"""Real agent workload driven by the repository's local MaaS replay server."""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import BinaryIO


class AgentWorkload:
    def __init__(
        self,
        repo_root: Path,
        work_dir: Path,
        binary: Path,
        turns: int = 4,
        tpot_ms: float = 1.0,
        https: bool = True,
        timeout_seconds: float = 10.0,
        kind: str = "xiaoo",
        setup_timeout_seconds: float = 3600.0,
        input_bytes: int | None = None,
        max_request_bytes: int = 16777216,
    ):
        if turns < 2 or tpot_ms < 0 or timeout_seconds <= 0:
            raise ValueError("agent turns must be >= 2, tpot >= 0, timeout > 0")
        self.repo_root = repo_root
        self.work_dir = work_dir
        self.binary = binary.resolve()
        self.kind = kind
        self.turns = turns
        self.tpot_ms = tpot_ms
        self.https = https
        self.timeout_seconds = timeout_seconds
        self.setup_timeout_seconds = setup_timeout_seconds
        if max_request_bytes <= 0:
            raise ValueError("MaaS request limit must be positive")
        self.max_request_bytes = max_request_bytes
        self.env: dict[str, str] = {}
        self._process: subprocess.Popen[bytes] | None = None
        self._log: BinaryIO | None = None
        self._log_offset = 0
        self._port = 0
        self._agent_port = 0
        self._fixture = json.loads(
            (Path(__file__).parent / "maas" / "agent.json").read_text()
        )
        line = self._fixture["input_line"].encode()
        length = self._fixture["input_bytes"] if input_bytes is None else input_bytes
        if length <= 0:
            raise ValueError("agent input bytes must be positive")
        self._input = (line * ((length + len(line) - 1) // len(line)))[:length]

    def start(self) -> None:
        if not self.binary.is_file() or not os.access(self.binary, os.X_OK):
            raise RuntimeError(f"{self.kind} is not executable: {self.binary}")
        if self.https and shutil.which("openssl") is None:
            raise RuntimeError("HTTPS agent benchmark requires openssl")
        self.work_dir.mkdir(parents=True, exist_ok=True)
        if self.kind == "opencode":
            self._prepare_opencode_dependencies()
        templates = self.work_dir / "templates"
        templates.mkdir()
        self._write_scenario(templates)
        server = self.repo_root / "tests/v2/common/test_suites/local_maas_server/server.py"
        if not server.is_file():
            raise RuntimeError(f"local MaaS server is missing: {server}")
        self._port = self._free_port()
        self._agent_port = self._free_port() if self.https else self._port
        while self.https and self._agent_port == self._port:
            self._agent_port = self._free_port()
        command = [
            sys.executable, str(server), "replay",
            "--http-bind-host", "127.0.0.1", "--http-bind-port", str(self._port),
            "--templates-dir", str(templates), "--scenario", self._fixture["name"],
            "--tpot-milliseconds", str(self.tpot_ms), "--log-requests",
            "--max-request-bytes", str(self.max_request_bytes),
        ]
        if self.https:
            command += [
                "--https-bind-port", str(self._agent_port),
                "--tls-work-dir", str(self.work_dir),
            ]
        else:
            command += ["--disable-https"]
        self._log = (self.work_dir / "maas.log").open("wb")
        try:
            self._process = subprocess.Popen(
                command, cwd=self.repo_root, stdout=self._log, stderr=subprocess.STDOUT,
            )
            deadline = time.monotonic() + self.timeout_seconds
            while time.monotonic() < deadline:
                if self._process.poll() is not None:
                    raise RuntimeError(f"local MaaS exited; see {self.work_dir / 'maas.log'}")
                try:
                    self._request("GET", "/healthz")
                    if self.https:
                        bundles = list(self.work_dir.glob("*/combined-ca.pem"))
                        if len(bundles) != 1:
                            time.sleep(0.05)
                            continue
                        self.env["SSL_CERT_FILE"] = str(bundles[0])
                        if self.kind == "opencode":
                            self.env["NODE_EXTRA_CA_CERTS"] = str(bundles[0])
                    return
                except (urllib.error.URLError, ConnectionError, TimeoutError):
                    time.sleep(0.05)
            raise RuntimeError(f"local MaaS startup timed out; see {self.work_dir / 'maas.log'}")
        except BaseException:
            self.stop()
            raise

    def reset(self) -> None:
        self._request("POST", "/reset")
        self._log_offset = (self.work_dir / "maas.log").stat().st_size

    def prepare(self, workdir: Path) -> None:
        workdir.mkdir(parents=True, exist_ok=True)
        (workdir / "input.txt").write_bytes(self._input)
        if self.kind == "opencode":
            self._prepare_opencode(workdir)
            return
        (workdir / "xiaoo.toml").write_text("")
        (workdir / "mcp.json").write_text('{"mcpServers":{}}\n')

    def command(self, workdir: Path) -> list[str]:
        scheme = "https" if self.https else "http"
        if self.kind == "opencode":
            return [
                str(self.binary), "run", "--dir", str(workdir),
                "--auto", "--pure", "--format", "json",
                "--title", "Payload benchmark", "--model", "bench/deepseek-v4-flash",
                self._fixture["prompt"],
            ]
        return [
            str(self.binary), "--cli", "run",
            "--config", str(workdir / "xiaoo.toml"),
            "--mcp-config", str(workdir / "mcp.json"),
            "--prompt", self._fixture["prompt"],
            "--provider", "openai", "--api-key", "bench",
            "--api-base", f"{scheme}://127.0.0.1:{self._agent_port}/v1/chat/completions",
            "--model", "deepseek-v4-flash", "--max-turns", str(self.turns),
            "--tools", "bash", "--format", "json",
        ]

    def _prepare_opencode(self, workdir: Path) -> None:
        # Give snapshots a small, independent project instead of the parent repository.
        subprocess.run(["git", "init", "--quiet", str(workdir)], check=True, timeout=10)
        scheme = "https" if self.https else "http"
        config = workdir / "opencode.json"
        config.write_text(json.dumps({
            "provider": {"bench": {
                "npm": "@ai-sdk/openai-compatible",
                "options": {
                    "baseURL": f"{scheme}://127.0.0.1:{self._agent_port}/v1",
                    "apiKey": "bench",
                },
                "models": {"deepseek-v4-flash": {"name": "deepseek-v4-flash"}},
            }},
            "agent": {"title": {"disable": True}},
        }, indent=2) + "\n")
        self.env["OPENCODE_CONFIG"] = str(config)

    def _prepare_opencode_dependencies(self) -> None:
        # --pure skips plugin loading, but configuration still starts npm in the background.
        # Complete that work before measurement and reuse the real installation.
        state = self.work_dir / "opencode-state"
        for name in ("CONFIG", "DATA", "CACHE", "STATE"):
            directory = state / name.lower()
            directory.mkdir(parents=True, exist_ok=True)
            self.env[f"XDG_{name}_HOME"] = str(directory)
        self.env.update({
            "OPENCODE_TEST_HOME": str(state),
            "OPENCODE_DISABLE_PROJECT_CONFIG": "1",
            "OPENCODE_DISABLE_MODELS_FETCH": "1",
            "OPENCODE_DISABLE_AUTOUPDATE": "1",
        })
        env = dict(os.environ, **self.env)
        version = subprocess.run(
            [str(self.binary), "--version"], env=env, check=True, capture_output=True,
            text=True, timeout=self.timeout_seconds,
        ).stdout.strip()
        directory = state / "config/opencode"
        directory.mkdir(exist_ok=True)
        with (self.work_dir / "npm-setup.log").open("wb") as log:
            subprocess.run(
                ["npm", "install", "--prefix", str(directory), "--save-exact",
                 "--ignore-scripts", "--no-audit", "--no-fund",
                 f"@opencode-ai/plugin@{version}"],
                env=env, stdout=log, stderr=subprocess.STDOUT, check=True,
                timeout=self.setup_timeout_seconds,
            )
        package = json.loads((directory / "package.json").read_text())
        lock = json.loads((directory / "package-lock.json").read_text())
        installed = json.loads((directory / "node_modules/@opencode-ai/plugin/package.json").read_text())
        if (package["dependencies"]["@opencode-ai/plugin"] != version
                or lock["packages"][""]["dependencies"]["@opencode-ai/plugin"] != version
                or installed["version"] != version):
            raise RuntimeError(f"OpenCode dependency installation does not match {version}")
        self.env["npm_config_offline"] = "true"

    def validate(self, workdir: Path, output_path: Path) -> dict[str, object]:
        if self._fixture["completion"] not in output_path.read_text(errors="replace"):
            raise RuntimeError(f"agent did not reach its final response: {output_path}")
        for index in range(1, self.turns):
            result = workdir / f"result-{index}.txt"
            if not result.is_file() or result.read_bytes() != self._input:
                raise RuntimeError(f"agent tool output is missing or incorrect: {result}")
        # Request completion logging occurs at the end of the HTTP handler.
        deadline = time.monotonic() + 1.0
        count = 0
        while time.monotonic() < deadline:
            with (self.work_dir / "maas.log").open("rb") as stream:
                stream.seek(self._log_offset)
                records = stream.read().decode(errors="replace").splitlines()
            count = 0
            for line in records:
                if not line.startswith("{"):
                    continue
                record = json.loads(line)
                if record.get("event") == "local_maas_request":
                    if record["status"] != 200:
                        raise RuntimeError(f"local MaaS request failed: {record['status']}")
                    count += 1
            if count >= self.turns:
                break
            time.sleep(0.01)
        if count != self.turns:
            raise RuntimeError(f"expected {self.turns} LLM requests, observed {count}")
        return {
            "llm_requests": count,
            "verified_tool_outputs": self.turns - 1,
            "tool_output_bytes_each": len(self._input),
            "transport": "https" if self.https else "http",
        }

    def stop(self) -> None:
        if self._process is not None and self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=5)
        if self._log is not None:
            self._log.close()
            self._log = None

    def cleanup(self, workdir: Path) -> None:
        if self.kind == "opencode":
            shutil.rmtree(workdir / ".git")

    def _write_scenario(self, directory: Path) -> None:
        generators = [
            {
                "type": "response",
                "response": {
                    "blocks": [{
                        "type": "tool_call", "name": "bash",
                        "arguments": {"command": self._fixture["tool_command"].format(index=index)},
                    }],
                    "usage": {"output_tokens": self._fixture["output_tokens"]},
                },
            }
            for index in range(1, self.turns)
        ]
        generators.append({
            "type": "response",
            "response": {
                "blocks": [{"type": "message", "text": self._fixture["completion"]}],
                "usage": {"output_tokens": self._fixture["output_tokens"]},
            },
        })
        name = self._fixture["name"]
        (directory / f"{name}.seq.json").write_text(json.dumps({
            "type": "sequential", "generators": generators,
        }))
        (directory / f"{name}.meta.json").write_text(json.dumps({
            "name": name, "description": "Real agent reads and returns deterministic local tool output.",
            "type": "sequential", "infinite": False, "sequence": f"{name}.seq.json",
            "rounds": self.turns, "tool_rounds": self.turns - 1, "message_rounds": 1,
            "tools": ["bash"],
        }))

    def _request(self, method: str, path: str) -> None:
        request = urllib.request.Request(f"http://127.0.0.1:{self._port}{path}", method=method)
        with urllib.request.urlopen(request, timeout=1) as response:
            if response.status != 200:
                raise RuntimeError(f"local MaaS {path} returned {response.status}")

    @staticmethod
    def _free_port() -> int:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.bind(("127.0.0.1", 0))
            return int(sock.getsockname()[1])
