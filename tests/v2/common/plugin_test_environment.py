from __future__ import annotations

import copy
import json
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, TextIO

from .actrail_runtime import ActrailRuntime
from .config import CommonTestConfig
from .output import TestOutput
from .plugin_web_api import PluginWebApi
from .test_case import TestResult, TestStatus


@dataclass(frozen=True)
class PluginRuntimeSpec:
    package: str
    instance_id: str
    plugin_id: str
    runtime: str


class PluginTestEnvironment:
    """Reusable actraild + actrailweb + runtime-plugin test lifecycle."""

    def __init__(
        self,
        config: CommonTestConfig,
        output: TestOutput,
        *,
        operator_config: Path,
        operator_config_patch: Path | None = None,
        web_host: str,
        web_port: int,
        plugin: PluginRuntimeSpec,
    ):
        self.config = config
        self.plugin = plugin
        self.operator_config = operator_config
        self.runtime = ActrailRuntime(
            config.repo,
            config.bin_dir,
            config.command_timeout_seconds,
            output,
            operator_config,
            operator_config_patch,
        )
        bin_dir = (
            config.bin_dir
            if config.bin_dir.is_absolute()
            else config.repo / config.bin_dir
        )
        self._actrailweb = bin_dir / "actrailweb"
        if not self._actrailweb.is_file():
            raise RuntimeError(f"release binary not found: {self._actrailweb}")
        self.api = PluginWebApi(
            f"http://{web_host}:{web_port}",
            config.command_timeout_seconds,
        )
        self._web_host = web_host
        self._web_port = web_port
        self._web_process: subprocess.Popen[str] | None = None
        self._web_log: TextIO | None = None
        self._daemon_started = False
        self._plugin_loaded = False
        self._original_config: dict[str, Any] | None = None

    def prepare(self) -> dict[str, Any]:
        if not self.config.work_dir.is_dir():
            raise RuntimeError(
                f"runner did not prepare case work directory: {self.config.work_dir}"
            )
        self.runtime.prepare()
        self._daemon_started = True
        self._start_web()
        catalog = self._wait_for_catalog()
        self._require_catalog_package(catalog)
        self._load_plugin()
        config_document = self.api.config(self.plugin.instance_id)
        raw_config = config_document.get("config")
        if not isinstance(raw_config, dict):
            raise AssertionError("plugin config response has no config object")
        self._original_config = copy.deepcopy(raw_config)
        (self.config.work_dir / "config-initial.json").write_text(
            json.dumps(config_document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return config_document

    def current_config(self) -> dict[str, Any]:
        document = self.api.config(self.plugin.instance_id)
        config = document.get("config")
        if not isinstance(config, dict):
            raise AssertionError("plugin config response has no config object")
        return copy.deepcopy(config)

    def update_config(self, candidate: dict[str, Any]) -> dict[str, Any]:
        validation = self.api.validate_config(self.plugin.instance_id, candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"plugin config validation failed: {validation}")
        updated = self.api.update_config(self.plugin.instance_id, candidate)
        returned = updated.get("config")
        if not isinstance(returned, dict):
            raise AssertionError("plugin config update returned no config object")
        return returned

    def cleanup(self) -> TestResult:
        failures: list[str] = []
        if self._plugin_loaded and self._original_config is not None:
            try:
                self.api.update_config(
                    self.plugin.instance_id,
                    self._original_config,
                )
            except Exception as error:
                failures.append(f"restore plugin config: {error}")
        if self._plugin_loaded:
            try:
                self.api.unload(self.plugin.instance_id)
                self._plugin_loaded = False
            except Exception as error:
                failures.append(f"unload plugin: {error}")
        self._stop_web(failures)
        if self._daemon_started:
            stopped = self.runtime.stop()
            if stopped is None or stopped.returncode != 0:
                returncode = None if stopped is None else stopped.returncode
                failures.append(f"stop actraild: returncode={returncode}")
            cleaned = self.runtime.clean(echo=False)
            if cleaned.returncode != 0:
                failures.append(
                    f"clean traces: returncode={cleaned.returncode} "
                    f"stderr={cleaned.stderr[-1000:]}"
                )
        if failures:
            return TestResult(TestStatus.FAILED, "; ".join(failures))
        return TestResult(
            TestStatus.PASSED,
            "plugin config restored, services stopped, and traces cleaned",
        )

    def _start_web(self) -> None:
        self._web_log = (self.config.work_dir / "actrailweb.log").open(
            "w",
            encoding="utf-8",
        )
        self._web_process = subprocess.Popen(
            [
                str(self._actrailweb),
                "--config",
                str(self.operator_config),
                "--addr",
                self._web_host,
                "--port",
                str(self._web_port),
            ],
            cwd=self.config.repo,
            stdout=self._web_log,
            stderr=subprocess.STDOUT,
            text=True,
        )

    def _wait_for_catalog(self) -> dict[str, Any]:
        last_error: Exception | None = None
        for _ in range(self.config.drain_attempts):
            if self._web_process is not None and self._web_process.poll() is not None:
                raise RuntimeError(
                    f"actrailweb exited early with {self._web_process.returncode}"
                )
            try:
                catalog = self.api.catalog()
                if (
                    catalog.get("available") is True
                    and catalog.get("runtime_available") is True
                ):
                    return catalog
            except Exception as error:
                last_error = error
            time.sleep(self.config.drain_interval_seconds)
        raise RuntimeError(f"plugin catalog did not become ready: {last_error}")

    def _require_catalog_package(self, catalog: dict[str, Any]) -> None:
        packages = catalog.get("packages")
        if not isinstance(packages, list):
            raise AssertionError("plugin catalog has no packages array")
        found = any(
            isinstance(item, dict)
            and item.get("package_key") == self.plugin.package
            and item.get("plugin_id") == self.plugin.plugin_id
            and item.get("runtime") == self.plugin.runtime
            and item.get("activation_ready") is True
            for item in packages
        )
        if not found:
            raise AssertionError(
                f"catalog has no activation-ready {self.plugin.package} package"
            )

    def _load_plugin(self) -> None:
        loaded = self.api.load(self.plugin.package, self.plugin.instance_id)
        self._plugin_loaded = True
        plugin = loaded.get("plugin")
        if not isinstance(plugin, dict):
            raise AssertionError(f"plugin load response has no plugin: {loaded}")
        expected = {
            "instance_id": self.plugin.instance_id,
            "plugin_id": self.plugin.plugin_id,
            "runtime": self.plugin.runtime,
            "state": "active",
        }
        for key, value in expected.items():
            if plugin.get(key) != value:
                raise AssertionError(
                    f"loaded plugin {key}={plugin.get(key)!r}, expected {value!r}"
                )

    def _stop_web(self, failures: list[str]) -> None:
        if self._web_process is not None:
            if self._web_process.poll() is None:
                self._web_process.terminate()
                try:
                    self._web_process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self._web_process.kill()
                    self._web_process.wait(timeout=10)
            if self._web_process.returncode not in (0, -15):
                failures.append(
                    f"actrailweb exited with {self._web_process.returncode}"
                )
            self._web_process = None
        if self._web_log is not None:
            self._web_log.close()
            self._web_log = None
