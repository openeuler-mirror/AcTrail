from __future__ import annotations

import subprocess
import time
from pathlib import Path
from urllib.parse import urlsplit

from tests.v2.common.plugin_web_api import PluginWebApi


ALERT_FORWARDING_INSTANCE = "builtin.alert-forwarding"


class AlertForwardingWebControl:
    """Own actrailweb while exercising the builtin forwarding control API."""

    def __init__(
        self,
        executable: Path,
        operator_config: Path,
        host: str,
        port: int,
        work_dir: Path,
        command_timeout_seconds: int,
    ):
        self._executable = executable.resolve()
        self._operator_config = operator_config.resolve()
        self._host = host
        self._port = port
        self._log_path = work_dir / "actrailweb.log"
        self._command_timeout_seconds = command_timeout_seconds
        self._process: subprocess.Popen[str] | None = None
        self._log = None
        self.api: PluginWebApi | None = None

    def start(self, timeout_seconds: float) -> None:
        if not self._executable.is_file():
            raise RuntimeError(f"release binary not found: {self._executable}")
        self._log = self._log_path.open("w", encoding="utf-8")
        self._process = subprocess.Popen(
            [
                str(self._executable),
                "--config",
                str(self._operator_config),
                "--addr",
                self._host,
                "--port",
                str(self._port),
            ],
            stdout=self._log,
            stderr=subprocess.STDOUT,
            text=True,
        )
        deadline = time.monotonic() + timeout_seconds
        prefix = "actrailweb listening on "
        while time.monotonic() < deadline:
            if self._process.poll() is not None:
                raise RuntimeError(
                    f"actrailweb exited with {self._process.returncode}: {self.log_tail()}"
                )
            for line in self._log_path.read_text(encoding="utf-8").splitlines():
                if not line.startswith(prefix):
                    continue
                url = line.removeprefix(prefix).split()[0]
                parsed = urlsplit(url)
                if parsed.scheme != "http" or parsed.hostname is None or parsed.port is None:
                    raise RuntimeError(f"actrailweb reported invalid URL: {url}")
                self.api = PluginWebApi(url, self._command_timeout_seconds)
                return
            time.sleep(0.05)
        raise RuntimeError(f"actrailweb did not become ready: {self.log_tail()}")

    def configure(self, *, enabled: bool, categories: list[str]) -> dict:
        api = self._require_api()
        candidate = {
            "enabled": enabled,
            "all_categories": False,
            "categories": categories,
        }
        validation = api.validate_config(ALERT_FORWARDING_INSTANCE, candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"forwarding config validation failed: {validation}")
        response = api.update_config(ALERT_FORWARDING_INSTANCE, candidate)
        config = response.get("config")
        if config != candidate:
            raise AssertionError(
                f"forwarding config update was not retained: {config} != {candidate}"
            )
        return config

    def config(self) -> dict:
        document = self._require_api().config(ALERT_FORWARDING_INSTANCE)
        config = document.get("config")
        if not isinstance(config, dict):
            raise AssertionError(f"forwarding config response is invalid: {document}")
        return config

    def stop(self) -> None:
        process = self._process
        self._process = None
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if self._log is not None:
            self._log.close()
            self._log = None
        self.api = None

    def log_tail(self) -> str:
        try:
            return self._log_path.read_text(encoding="utf-8")[-2000:]
        except OSError:
            return "<unavailable>"

    def _require_api(self) -> PluginWebApi:
        if self.api is None:
            raise RuntimeError("actrailweb is not running")
        return self.api
