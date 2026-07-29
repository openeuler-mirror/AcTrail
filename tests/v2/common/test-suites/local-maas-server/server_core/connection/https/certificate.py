from __future__ import annotations

import ipaddress
import os
import re
import secrets
import shutil
import ssl
import subprocess
import tempfile
from pathlib import Path

from server_core.connection.interface import ConnectionStartupError

from .config import HTTPSConfig


class EphemeralCertificate:
    def __init__(self, config: HTTPSConfig):
        parent = config.certificate_work_dir
        if parent is not None:
            parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if not parent.is_dir():
                raise ConnectionStartupError(
                    f"TLS work path is not a directory: {parent}"
                )
        self._directory = tempfile.TemporaryDirectory(
            prefix="local-maas-tls-",
            dir=None if parent is None else str(parent),
        )
        directory = Path(self._directory.name)
        self._local_ca_cert_file = directory / "local-ca.pem"
        self.ca_cert_file = directory / "combined-ca.pem"
        self.server_cert_file = directory / "server.pem"
        self.server_key_file = directory / "server-key.pem"
        try:
            self._generate(config, directory)
            self._combine_ca_bundle()
        except Exception:
            self.close()
            raise

    def close(self) -> None:
        self._directory.cleanup()

    def _generate(self, config: HTTPSConfig, directory: Path) -> None:
        openssl = self._resolve_openssl(config.openssl_binary)
        ca_key = directory / "ca-key.pem"
        server_request = directory / "server.csr"
        extension_file = directory / "server.ext"
        extension_file.write_text(
            "\n".join(
                (
                    "basicConstraints=critical,CA:FALSE",
                    "keyUsage=critical,digitalSignature,keyEncipherment",
                    "extendedKeyUsage=serverAuth",
                    f"subjectAltName={self._subject_alt_names(config.bind_host)}",
                    "",
                )
            ),
            encoding="utf-8",
        )

        self._run(
            openssl,
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-nodes",
            "-days",
            str(config.certificate_validity_days),
            "-subj",
            "/CN=Local MaaS Test CA",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-keyout",
            str(ca_key),
            "-out",
            str(self._local_ca_cert_file),
        )
        self._run(
            openssl,
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            str(self.server_key_file),
            "-out",
            str(server_request),
        )
        serial = secrets.randbits(159) | 1
        self._run(
            openssl,
            "x509",
            "-req",
            "-sha256",
            "-days",
            str(config.certificate_validity_days),
            "-in",
            str(server_request),
            "-CA",
            str(self._local_ca_cert_file),
            "-CAkey",
            str(ca_key),
            "-set_serial",
            f"0x{serial:x}",
            "-extfile",
            str(extension_file),
            "-out",
            str(self.server_cert_file),
        )
        os.chmod(self.server_key_file, 0o600)
        ca_key.unlink()
        server_request.unlink()
        extension_file.unlink()

    def _combine_ca_bundle(self) -> None:
        configured_bundle = os.environ.get("SSL_CERT_FILE")
        if configured_bundle:
            base_bundle = Path(configured_bundle).expanduser()
        else:
            default_bundle = ssl.get_default_verify_paths().cafile
            if default_bundle is None:
                raise ConnectionStartupError(
                    "system CA bundle was not found"
                )
            base_bundle = Path(default_bundle)
        if not base_bundle.is_file():
            raise ConnectionStartupError(
                f"CA bundle does not exist: {base_bundle}"
            )
        with (
            base_bundle.open("rb") as base_file,
            self._local_ca_cert_file.open("rb") as local_file,
            self.ca_cert_file.open("wb") as combined_file,
        ):
            shutil.copyfileobj(base_file, combined_file)
            combined_file.write(b"\n")
            shutil.copyfileobj(local_file, combined_file)

    @staticmethod
    def _resolve_openssl(configured: str) -> str:
        resolved = shutil.which(configured)
        if resolved is None:
            raise ConnectionStartupError(
                f"OpenSSL executable not found: {configured}"
            )
        return resolved

    @staticmethod
    def _subject_alt_names(bind_host: str) -> str:
        names = ["DNS:localhost", "IP:127.0.0.1", "IP:::1"]
        try:
            address = ipaddress.ip_address(bind_host)
        except ValueError:
            if (
                bind_host != "localhost"
                and re.fullmatch(r"[A-Za-z0-9.-]+", bind_host)
            ):
                names.append(f"DNS:{bind_host}")
        else:
            if not address.is_unspecified and str(address) not in {
                "127.0.0.1",
                "::1",
            }:
                names.append(f"IP:{address}")
        return ",".join(names)

    @staticmethod
    def _run(
        openssl: str,
        *arguments: str,
    ) -> None:
        try:
            completed = subprocess.run(
                (openssl, *arguments),
                capture_output=True,
                text=True,
                check=False,
            )
        except OSError as error:
            raise ConnectionStartupError(
                f"OpenSSL certificate command failed: {error}"
            ) from error
        if completed.returncode != 0:
            detail = completed.stderr.strip()[-2000:]
            raise ConnectionStartupError(
                "OpenSSL certificate command returned "
                f"{completed.returncode}: {detail}"
            )
