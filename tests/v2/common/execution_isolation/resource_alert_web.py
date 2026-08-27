from __future__ import annotations

import subprocess
import time
from pathlib import Path
from urllib.parse import urlsplit

from tests.v2.common.plugin_web_api import PluginWebApi


class SandboxResourceAlertWebControl:
    """Own actrailweb and operate one sandbox resource alert instance."""

    def __init__(
        self,
        executable: Path,
        operator_config: Path,
        port: int,
        work_dir: Path,
        timeout_seconds: int,
        instance_id: str,
    ) -> None:
        self._executable = executable.resolve()
        self._operator_config = operator_config.resolve()
        self._port = port
        self._log_path = work_dir / "actrailweb-sandbox-resource.log"
        self._timeout_seconds = timeout_seconds
        self._instance_id = instance_id
        self._process: subprocess.Popen[str] | None = None
        self._log = None
        self._api: PluginWebApi | None = None

    def start(self, timeout_seconds: float) -> None:
        self._log = self._log_path.open("w", encoding="utf-8")
        self._process = subprocess.Popen(
            [
                str(self._executable),
                "--config",
                str(self._operator_config),
                "--addr",
                "127.0.0.1",
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
                raise RuntimeError(f"actrailweb exited: {self.log_tail()}")
            for line in self._log_path.read_text(encoding="utf-8").splitlines():
                if not line.startswith(prefix):
                    continue
                parsed = urlsplit(line.removeprefix(prefix).split()[0])
                if parsed.scheme != "http" or parsed.netloc == "":
                    raise RuntimeError(f"actrailweb reported invalid address: {line}")
                self._api = PluginWebApi(parsed.geturl(), self._timeout_seconds)
                return
            time.sleep(0.05)
        raise RuntimeError(f"actrailweb did not become ready: {self.log_tail()}")

    def update_memory_threshold(self, threshold_bytes: int) -> dict:
        api = self._require_api()
        document = api.config(self._instance_id)
        config = document.get("config")
        if not isinstance(config, dict):
            raise AssertionError(f"resource alert config is invalid: {document}")
        candidate = dict(config)
        candidate["memory_available_threshold_bytes"] = threshold_bytes
        validation = api.validate_config(self._instance_id, candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"resource alert config validation failed: {validation}")
        updated = api.update_config(self._instance_id, candidate).get("config")
        if updated != candidate:
            raise AssertionError(f"resource alert config update was not retained: {updated}")
        return candidate

    def assert_memory_update_rejected(self, threshold_bytes: int) -> None:
        api = self._require_api()
        before = api.config(self._instance_id).get("config")
        if not isinstance(before, dict):
            raise AssertionError(f"resource alert config is invalid: {before}")
        candidate = dict(before)
        candidate["memory_available_threshold_bytes"] = threshold_bytes
        try:
            api.update_config(self._instance_id, candidate)
        except RuntimeError:
            pass
        else:
            raise AssertionError("resource alert config update unexpectedly succeeded")
        after = api.config(self._instance_id).get("config")
        if after != before:
            raise AssertionError("rejected resource alert config became active")

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
        self._api = None

    def log_tail(self) -> str:
        try:
            return self._log_path.read_text(encoding="utf-8")[-2000:]
        except OSError:
            return "<unavailable>"

    def _require_api(self) -> PluginWebApi:
        if self._api is None:
            raise RuntimeError("actrailweb is not running")
        return self._api
