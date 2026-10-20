"""Real upstream TLS relay for plaintext WebSocket capture acceptance.

This exercises protocol ownership through a loopback socket. It does not establish
that native Rustls pointer-based capture has a stable connection identity.
No response is generated locally, and request/response content is never logged.
"""

import socket
import ssl
import threading
import time
import base64
from urllib.parse import unquote, urlsplit
from urllib.request import getproxies, proxy_bypass


class TlsRelay:
    def __init__(
        self,
        upstream_url="https://chatgpt.com/backend-api/codex",
        timeout_seconds=30,
        *,
        header_limit_bytes=65536,
        max_connections=16,
        read_size_bytes=65536,
        cleanup_timeout_seconds=5,
    ):
        parsed = urlsplit(upstream_url)
        if parsed.scheme != "https" or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("upstream must be an HTTPS URL without credentials, query or fragment")
        upstream_host = parsed.hostname
        upstream_port = parsed.port or 443
        if not upstream_host or any(c in upstream_host for c in "\r\n /\t"):
            raise ValueError("invalid upstream host")
        for value in (header_limit_bytes, timeout_seconds, max_connections,
                      read_size_bytes, cleanup_timeout_seconds):
            if value <= 0:
                raise ValueError("relay limits must be positive")
        if not 0 < upstream_port < 65536:
            raise ValueError("invalid upstream port")
        self.upstream_host = upstream_host
        self.upstream_port = upstream_port
        self._base_path = parsed.path.rstrip("/")
        self.header_limit_bytes = header_limit_bytes
        self.io_timeout_seconds = timeout_seconds
        self.read_size_bytes = read_size_bytes
        self.cleanup_timeout_seconds = cleanup_timeout_seconds
        self._slots = threading.BoundedSemaphore(max_connections)
        self._lock = threading.Lock()
        self._sockets = set()
        self._threads = set()
        self._stop = threading.Event()
        self._listener = None
        self._context = ssl.create_default_context()
        proxies = getproxies()
        self._proxy = None
        if not proxy_bypass(upstream_host):
            proxy_url = proxies.get("https") or proxies.get("http")
            if proxy_url:
                proxy = urlsplit(proxy_url)
                if proxy.scheme != "http" or not proxy.hostname:
                    raise ValueError("TLS relay requires an HTTP CONNECT proxy")
                self._proxy = proxy

    @property
    def base_url(self):
        if self._listener is None:
            raise RuntimeError("relay is not running")
        return f"http://127.0.0.1:{self._listener.getsockname()[1]}{self._base_path}"

    @property
    def ws_base_url(self):
        return self.base_url.replace("http://", "ws://", 1)

    def __enter__(self):
        if self._listener is not None or self._stop.is_set():
            raise RuntimeError("relay context cannot be reused")
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self._listener.bind(("127.0.0.1", 0))
            self._listener.listen()
            self._listener.settimeout(min(0.25, self.io_timeout_seconds))
            self._spawn(self._accept)
        except BaseException:
            self.close()
            raise
        return self

    def __exit__(self, *_):
        self.close()

    def _spawn(self, target, *args):
        def run():
            try:
                target(*args)
            finally:
                with self._lock:
                    self._threads.discard(threading.current_thread())

        thread = threading.Thread(target=run, daemon=True)
        with self._lock:
            self._threads.add(thread)
        thread.start()
        return thread

    def _track(self, sock):
        with self._lock:
            if self._stop.is_set():
                sock.close()
                raise OSError("relay stopped")
            self._sockets.add(sock)
        return sock

    def _close_socket(self, sock):
        if sock is None:
            return
        try:
            sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        sock.close()
        with self._lock:
            self._sockets.discard(sock)

    def _accept(self):
        while not self._stop.is_set():
            try:
                client, _ = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            if not self._slots.acquire(blocking=False):
                client.close()
                continue
            try:
                self._track(client)
            except OSError:
                self._slots.release()
                break
            self._spawn(self._connection, client)

    def _initial_request(self, client):
        data = bytearray()
        deadline = time.monotonic() + self.io_timeout_seconds
        while b"\r\n\r\n" not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or len(data) >= self.header_limit_bytes:
                raise OSError("initial HTTP header limit reached")
            client.settimeout(remaining)
            chunk = client.recv(min(self.read_size_bytes, self.header_limit_bytes - len(data)))
            if not chunk:
                raise OSError("closed before initial HTTP headers")
            data.extend(chunk)
        head, body = bytes(data).split(b"\r\n\r\n", 1)
        lines = head.split(b"\r\n")
        host = self.upstream_host
        if self.upstream_port != 443:
            host += f":{self.upstream_port}"
        replacement = b"Host: " + host.encode("ascii")
        found = False
        for index in range(1, len(lines)):
            if lines[index].split(b":", 1)[0].lower() == b"host":
                if found:
                    raise OSError("multiple Host headers")
                lines[index] = replacement
                found = True
        if not found:
            raise OSError("missing Host header")
        client.settimeout(self.io_timeout_seconds)
        return b"\r\n".join(lines) + b"\r\n\r\n" + body

    def _pump(self, source, destination):
        try:
            while not self._stop.is_set():
                data = source.recv(self.read_size_bytes)
                if not data:
                    break
                destination.sendall(data)
        except (OSError, ssl.SSLError):
            pass
        finally:
            self._close_socket(source)
            self._close_socket(destination)

    def _connection(self, client):
        upstream = None
        reverse = None
        try:
            initial = self._initial_request(client)
            raw = self._connect_upstream()
            upstream = raw
            # Transfer ownership without allowing cleanup to lose the live TLS socket.
            with self._lock:
                if self._stop.is_set():
                    raise OSError("relay stopped")
                upstream = self._context.wrap_socket(
                    raw, server_hostname=self.upstream_host,
                    do_handshake_on_connect=False,
                )
                self._sockets.discard(raw)
                self._sockets.add(upstream)
            upstream.do_handshake()
            upstream.sendall(initial)
            reverse = self._spawn(self._pump, upstream, client)
            self._pump(client, upstream)
        except (OSError, ssl.SSLError):
            pass
        finally:
            self._close_socket(client)
            self._close_socket(upstream)
            if reverse is not None:
                reverse.join(self.cleanup_timeout_seconds)
            self._slots.release()

    def _connect_upstream(self):
        proxy = self._proxy
        target = (proxy.hostname, proxy.port or 80) if proxy else (
            self.upstream_host, self.upstream_port
        )
        raw = self._track(socket.create_connection(target, self.io_timeout_seconds))
        try:
            if proxy is not None:
                authority = f"{self.upstream_host}:{self.upstream_port}"
                lines = [f"CONNECT {authority} HTTP/1.1", f"Host: {authority}"]
                if proxy.username is not None:
                    credentials = f"{unquote(proxy.username)}:{unquote(proxy.password or '')}"
                    token = base64.b64encode(credentials.encode()).decode("ascii")
                    lines.append(f"Proxy-Authorization: Basic {token}")
                raw.sendall(("\r\n".join(lines) + "\r\n\r\n").encode("ascii"))
                # Read only the CONNECT header; never consume tunnel bytes.
                response = bytearray()
                deadline = time.monotonic() + self.io_timeout_seconds
                while not response.endswith(b"\r\n\r\n"):
                    remaining = deadline - time.monotonic()
                    if remaining <= 0 or len(response) >= self.header_limit_bytes:
                        raise OSError("proxy response header limit reached")
                    raw.settimeout(remaining)
                    byte = raw.recv(1)
                    if not byte:
                        raise OSError("proxy closed before CONNECT response")
                    response.extend(byte)
                status = bytes(response).split(b"\r\n", 1)[0].split(b" ", 2)
                if len(status) < 2 or status[1] != b"200":
                    raise OSError("proxy CONNECT failed")
                raw.settimeout(self.io_timeout_seconds)
            return raw
        except BaseException:
            self._close_socket(raw)
            raise

    def close(self):
        self._stop.set()
        listener = self._listener
        if listener is not None:
            listener.close()
        with self._lock:
            sockets = list(self._sockets)
        for sock in sockets:
            self._close_socket(sock)
        deadline = time.monotonic() + self.cleanup_timeout_seconds
        while True:
            with self._lock:
                threads = list(self._threads)
            if not threads or time.monotonic() >= deadline:
                break
            for thread in threads:
                thread.join(max(0, deadline - time.monotonic()))
        self._listener = None
