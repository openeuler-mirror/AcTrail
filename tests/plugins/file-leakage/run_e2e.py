#!/usr/bin/env python3
"""Exercise the official file-leakage component through daemon, SQLite, and Web."""

from __future__ import annotations

import json
import os
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
DEFAULT_RUN_DIR = ROOT / "temp/file-leakage-plugin-e2e"
TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
PLUGIN_INSTANCE = "actrail.file-leakage"
ALERT_DEFINITION_KEY = "file-leakage"
DEFAULT_ALERT_DEADLINE_SECONDS = 30.0
DEFAULT_LAUNCH_TIMEOUT_SECONDS = 30.0


class FileLeakageE2E:
    def __init__(self, run_dir: Path = DEFAULT_RUN_DIR) -> None:
        self.run_dir = run_dir.resolve()
        self.home_dir = self.run_dir / "home"
        self.plugin_root = self.home_dir / ".actrail/plugins"
        self.package_dir = self.plugin_root / "file-leakage"
        self.transient_package_dir = self.plugin_root / "file-leakage-transient"
        self.plugin_source = ROOT / "examples/plugins/wit-component/file-leakage"
        self.plugin_artifact = (
            self.plugin_source
            / "target/wasm32-wasip2/release/actrail_file_leakage_plugin.wasm"
        )
        self.config = self.run_dir / "operator.conf"
        self.patch = self.run_dir / "operator.patch.toml"
        self.socket_path = self.run_dir / "actraild.sock"
        self.database = self.run_dir / "actrail.sqlite"
        self.bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release")).resolve()
        self.actraild = self.require_binary("actraild")
        self.actrailctl = self.require_binary("actrailctl")
        self.actrailweb = self.require_binary("actrailweb")
        self.alert_deadline_seconds = positive_float_env(
            "ACTRAIL_E2E_ALERT_DEADLINE_SECONDS", DEFAULT_ALERT_DEADLINE_SECONDS
        )
        self.launch_timeout_seconds = positive_float_env(
            "ACTRAIL_E2E_LAUNCH_TIMEOUT_SECONDS", DEFAULT_LAUNCH_TIMEOUT_SECONDS
        )
        self.web_port = available_loopback_port()
        self.base_url = f"http://127.0.0.1:{self.web_port}"
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.daemon: subprocess.Popen[str] | None = None
        self.web: subprocess.Popen[str] | None = None
        self.daemon_log = None
        self.web_log = None
        self.created_files: list[Path] = []

    def run(self) -> None:
        require_root()
        self.prepare_runtime()
        try:
            self.start_services()
            self.require_catalog_lifecycle_and_load()
            in_scope_trace = self.launch(
                "file-leak-in-scope",
                ["/bin/sh", "-c", "printf in-scope > source.rs"],
            )
            self.require_completed_without_alert(in_scope_trace)

            deleted_path = (self.run_dir / "deleted-outside-workdir.tmp").resolve()
            deleted_trace = self.launch(
                "file-leak-deleted",
                [
                    "/bin/sh",
                    "-c",
                    f"printf deleted > {shell_quote(deleted_path)} && rm -- {shell_quote(deleted_path)}",
                ],
            )
            self.require_completed_without_alert(deleted_trace)
            if deleted_path.exists():
                raise RuntimeError(f"deleted workload unexpectedly left {deleted_path}")

            no_write_path = self.new_retained_path("created-without-write.tmp")
            no_write_trace = self.launch(
                "file-leak-created-without-write",
                ["/bin/sh", "-c", f"touch -- {shell_quote(no_write_path)}"],
            )
            self.require_completed_without_alert(no_write_trace)
            if not no_write_path.exists():
                raise RuntimeError(
                    f"created-without-write workload did not leave {no_write_path}"
                )

            retained_path = self.new_retained_path("retained-outside-workdir.tmp")
            retained_trace = self.launch_retained("file-leak-retained", retained_path)
            retained_alert = self.wait_for_trace_alert(retained_trace)
            self.require_alert(retained_alert, retained_trace, retained_path)
            self.require_trace_evicted(retained_trace)
            self.require_persisted_source_path(retained_trace, retained_path)
            self.require_trace_api_matches(retained_alert)
            self.require_detail_api_matches(retained_alert)
            self.require_storage_is_normalized(retained_trace, retained_path)
            self.require_limit_validation()

            newest = self.populate_latest_window(31)
            latest = self.get_json("/api/alerts")["alerts"]
            if len(latest) != 30:
                raise RuntimeError(f"default latest alert count is {len(latest)}, expected 30")
            latest_ids = [int(alert["alert_id"]) for alert in latest]
            if latest_ids != sorted(latest_ids, reverse=True):
                raise RuntimeError(f"latest alerts are not stably newest-first: {latest_ids}")
            if int(latest[0]["trace_id"]) != newest[0]:
                raise RuntimeError("latest alert does not belong to the newest generated trace")
            self.require_web_unload()

            print(f"file_leakage_retained_trace_id={retained_trace}")
            print(f"file_leakage_retained_alert_id={retained_alert['alert_id']}")
            print(f"file_leakage_retained_path={retained_path}")
            print(f"file_leakage_latest_default_count={len(latest)}")
            print("file leakage plugin e2e complete")
        finally:
            self.stop_services()
            self.cleanup()

    def require_binary(self, name: str) -> Path:
        path = self.bin_dir / name
        if not path.is_file() or not os.access(path, os.X_OK):
            raise RuntimeError(f"missing release binary {path}")
        return path

    def prepare_runtime(self) -> None:
        if self.run_dir.exists():
            shutil.rmtree(self.run_dir)
        self.plugin_root.mkdir(parents=True)
        (self.run_dir / "work").mkdir()
        if not self.plugin_artifact.is_file():
            raise RuntimeError(
                f"missing {self.plugin_artifact}; build the plugin with cargo build --release --target wasm32-wasip2"
            )
        self.patch.write_text(self.operator_patch(), encoding="utf-8")
        run_checked(
            [
                str(self.actraild),
                "--config",
                str(self.config),
                "init",
                "--force",
                "--patch",
                str(self.patch),
            ],
            cwd=ROOT,
        )

    def operator_patch(self) -> str:
        quote = json.dumps
        return f"""[control]
socket_path = {quote(str(self.socket_path))}
pending_connection_max = 32
active_trace_max = 128
pid_file = {quote(str(self.run_dir / 'actraild.pid'))}
log_path = {quote(str(self.run_dir / 'actraild.log'))}

[control.finalization]
traces_per_cycle = 8
poll_interval_ms = 25
settle_delay_ms = 250

[storage.sqlite]
path = {quote(str(self.database))}

[storage.retention]
enabled = false

[web]
listen_addr = "127.0.0.1:{self.web_port}"

[export.snapshot]
directory = {quote(str(self.run_dir / 'export'))}
payload_bytes_enabled = false
payload_text_enabled = false

[plugins.discovery]
directory = {quote(str(self.plugin_root))}
max_packages = 8
manifest_max_bytes = 262144

[plugins.startup]
enabled = false
failure_policy = "fail-fast"

[capture]
profile_name = "file-leakage-plugin-e2e"
capabilities = ["proc-lifecycle", "fs-access-basic"]
opportunistic_capabilities = []
disabled_capabilities = []

[payload.tls]
enabled = false
sync_event_socket_path = {quote(str(self.run_dir / 'tls-sync.sock'))}

[payload.stdio]
enabled = false

[payload.socket]
enabled = false

[seccomp_notify]
enabled = false

[process_seccomp]
enabled = false

[agent_invocation]
enabled = false

[application]
enabled = false
http1_enabled = false
http2_enabled = false

[application.http]
capture_host = false
sse_enabled = false
sse_data_policy = "disabled"

[resource_metrics]
enabled = false
"""

    def start_services(self) -> None:
        self.daemon_log = (self.run_dir / "daemon.stdout.log").open("w+", encoding="utf-8")
        self.daemon = subprocess.Popen(
            [str(self.actraild), "--config", str(self.config), "run"],
            cwd=ROOT,
            text=True,
            stdout=self.daemon_log,
            stderr=subprocess.STDOUT,
        )
        self.wait_for_daemon()
        self.web_log = (self.run_dir / "web.stdout.log").open("w+", encoding="utf-8")
        self.web = subprocess.Popen(
            [str(self.actrailweb), "--config", str(self.config)],
            cwd=ROOT,
            text=True,
            stdout=self.web_log,
            stderr=subprocess.STDOUT,
        )
        self.wait_for_web()

    def wait_for_daemon(self) -> None:
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            self.require_running(self.daemon, "actraild", self.daemon_log)
            if self.socket_path.exists():
                doctor = run_checked(
                    [str(self.actrailctl), "--config", str(self.config), "doctor"],
                    cwd=ROOT,
                    check=False,
                )
                if doctor.returncode == 0:
                    return
            time.sleep(0.05)
        raise RuntimeError(f"daemon did not become ready\n{self.read_log(self.daemon_log)}")

    def wait_for_web(self) -> None:
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            self.require_running(self.web, "actrailweb", self.web_log)
            try:
                self.get_json("/api/alerts")
                return
            except (OSError, RuntimeError, urllib.error.URLError):
                time.sleep(0.05)
        raise RuntimeError(f"web did not become ready\n{self.read_log(self.web_log)}")

    def require_catalog_lifecycle_and_load(self) -> None:
        initial = self.get_json("/api/plugins/catalog")
        self.require_catalog(initial, set())
        startup = self.get_json("/api/plugins/enabled")
        if startup.get("global_enabled") or startup.get("configured_count") != 0:
            raise RuntimeError(f"plugins unexpectedly enabled at startup: {startup}")

        self.install_package(self.package_dir)
        installed = self.get_json("/api/plugins/catalog")
        self.require_catalog(installed, {"file-leakage"})
        package = installed["packages"][0]
        if not package.get("activation_ready") or package.get("loaded_instances") != []:
            raise RuntimeError(f"installed plugin is not a loadable, unloaded package: {package}")

        self.install_package(self.transient_package_dir)
        added = self.get_json("/api/plugins/catalog")
        self.require_catalog(added, {"file-leakage", "file-leakage-transient"})
        shutil.rmtree(self.transient_package_dir)
        removed = self.get_json("/api/plugins/catalog")
        self.require_catalog(removed, {"file-leakage"})

        loaded = self.post_json("/api/plugins/catalog/load?package=file-leakage")
        status = loaded.get("plugin", {})
        if status.get("instance_id") != PLUGIN_INSTANCE or status.get("state") != "active":
            raise RuntimeError(f"Web did not load the discovered plugin: {loaded}")
        active = self.get_json("/api/plugins/catalog")
        self.require_catalog(active, {"file-leakage"})
        package = active["packages"][0]
        if package.get("loaded_instances") != [PLUGIN_INSTANCE]:
            raise RuntimeError(f"catalog did not join the loaded instance: {package}")

    def require_web_unload(self) -> None:
        shutil.rmtree(self.package_dir)
        missing = self.get_json("/api/plugins/catalog")
        self.require_catalog(missing, set())
        runtime_ids = {
            plugin.get("instance_id") for plugin in missing.get("runtime_plugins", [])
        }
        if runtime_ids != {PLUGIN_INSTANCE}:
            raise RuntimeError(
                f"loaded runtime disappeared with its installed package: {missing}"
            )
        unloaded = self.post_json(
            f"/api/plugins/runtime/unload?instance_id={PLUGIN_INSTANCE}"
        )
        status = unloaded.get("plugin", {})
        if status.get("instance_id") != PLUGIN_INSTANCE or status.get("state") == "active":
            raise RuntimeError(f"Web did not unload the plugin: {unloaded}")
        catalog = self.get_json("/api/plugins/catalog")
        if catalog.get("runtime_plugin_count") != 0:
            raise RuntimeError(f"runtime plugin remained after unload: {catalog}")
        self.require_catalog(catalog, set())

    def install_package(self, destination: Path) -> None:
        destination.mkdir(parents=True)
        for name in (
            "file-leakage.plugin.toml",
            "file-leakage.config.json",
            "file-leakage.config.v1.schema.json",
            "file-leakage.payload.v1.schema.json",
        ):
            shutil.copy2(self.plugin_source / name, destination / name)
        shutil.copy2(self.plugin_artifact, destination / self.plugin_artifact.name)

    def require_catalog(self, catalog: dict, expected_keys: set[str]) -> None:
        if not catalog.get("available"):
            raise RuntimeError(f"plugin catalog is unavailable: {catalog}")
        if Path(catalog.get("directory", "")) != self.plugin_root:
            raise RuntimeError(f"catalog scanned the wrong directory: {catalog}")
        actual_keys = {package["package_key"] for package in catalog.get("packages", [])}
        if actual_keys != expected_keys:
            raise RuntimeError(
                f"catalog packages are {sorted(actual_keys)}, expected {sorted(expected_keys)}"
            )
        if catalog.get("package_count") != len(expected_keys):
            raise RuntimeError(f"catalog package count is inconsistent: {catalog}")

    def launch(self, name: str, argv: list[str]) -> int:
        trace_id, _ = self.launch_with_output(name, argv)
        return trace_id

    def launch_with_output(self, name: str, argv: list[str]) -> tuple[int, str]:
        result = run_checked(
            [
                str(self.actrailctl),
                "--config",
                str(self.config),
                "launch",
                "--name",
                name,
                "--",
                *argv,
            ],
            cwd=self.run_dir / "work",
            timeout=self.launch_timeout_seconds,
        )
        match = TRACE_RE.search(result.stdout)
        if match is None:
            raise RuntimeError(f"could not parse trace id from launch output:\n{result.stdout}")
        return int(match.group(1)), result.stdout

    def launch_retained(self, name: str, path: Path) -> int:
        return self.launch(
            name,
            ["/bin/sh", "-c", f"printf retained >> {shell_quote(path)}"],
        )

    def new_retained_path(self, name: str) -> Path:
        path = (self.run_dir / name).resolve()
        if path.exists():
            path.unlink()
        self.created_files.append(path)
        return path

    def require_completed_without_alert(self, trace_id: int) -> None:
        deadline = time.monotonic() + self.alert_deadline_seconds
        idle_since: float | None = None
        while time.monotonic() < deadline:
            alerts = self.get_json(f"/api/traces/{trace_id}/alerts")["alerts"]
            if alerts:
                raise RuntimeError(f"trace {trace_id} unexpectedly produced alerts: {alerts}")
            if self.trace_is_evicted(trace_id) and self.plugin_is_idle():
                idle_since = idle_since or time.monotonic()
                if time.monotonic() - idle_since >= 0.5:
                    return
            else:
                idle_since = None
            time.sleep(0.05)
        raise RuntimeError(f"trace {trace_id} did not reach a completed no-alert state")

    def wait_for_trace_alert(self, trace_id: int) -> dict:
        deadline = time.monotonic() + self.alert_deadline_seconds
        while time.monotonic() < deadline:
            alerts = self.get_json(f"/api/traces/{trace_id}/alerts")["alerts"]
            if alerts:
                return alerts[0]
            self.require_running(self.daemon, "actraild", self.daemon_log)
            time.sleep(0.05)
        raise RuntimeError(
            f"timed out waiting for trace {trace_id} alert\n{self.read_log(self.daemon_log)}"
        )

    def require_alert(self, alert: dict, trace_id: int, path: Path) -> None:
        expected = {
            "trace_id": trace_id,
            "producer_plugin_id": "actrail.file-leakage",
            "definition_key": ALERT_DEFINITION_KEY,
            "kind": "file.leakage",
            "title": "存在文件泄露",
            "severity": "medium",
        }
        for field, value in expected.items():
            if alert.get(field) != value:
                raise RuntimeError(f"alert field {field}={alert.get(field)!r}, expected {value!r}")
        if alert.get("payload") != {"residual_files": [str(path)]}:
            raise RuntimeError(f"unexpected file-leakage payload: {alert.get('payload')}")
        forbidden = {"config", "metadata", "evidence", "actions", "alert_definition_id"}
        present = forbidden.intersection(alert)
        if present:
            raise RuntimeError(f"alert API leaked internal duplicate fields: {sorted(present)}")

    def require_trace_evicted(self, trace_id: int) -> None:
        if not self.trace_is_evicted(trace_id):
            raise RuntimeError(f"trace {trace_id} remains in active daemon memory after admission")

    def trace_is_evicted(self, trace_id: int) -> bool:
        result = run_checked(
            [str(self.actrailctl), "--config", str(self.config), "list-traces"],
            cwd=ROOT,
        )
        return f"trace-{trace_id}" not in result.stdout

    def plugin_is_idle(self) -> bool:
        result = run_checked(
            [
                str(self.actraild),
                "--config",
                str(self.config),
                "plugin",
                "status",
                "--instance",
                PLUGIN_INSTANCE,
            ],
            cwd=ROOT,
        )
        fields = parse_fields(result.stdout)
        last_error = fields.get("last_error", "none")
        if last_error not in ("", "none"):
            raise RuntimeError(f"file-leakage plugin failed: {last_error}")
        return fields.get("queue_depth") == "0"

    def require_persisted_source_path(self, trace_id: int, path: Path) -> None:
        with sqlite3.connect(self.database) as connection:
            count = connection.execute(
                "SELECT COUNT(*) FROM file_observation_paths WHERE trace_id = ? AND path = ?",
                (trace_id, str(path)),
            ).fetchone()[0]
        if count < 1:
            raise RuntimeError(f"trace {trace_id} did not persist source path {path}")

    def require_trace_api_matches(self, alert: dict) -> None:
        trace_id = int(alert["trace_id"])
        rows = self.get_json(f"/api/traces/{trace_id}/alerts")["alerts"]
        if alert not in rows:
            raise RuntimeError(f"trace alert API is missing the selected alert: {rows}")

    def require_detail_api_matches(self, alert: dict) -> None:
        detail = self.get_json(f"/api/alerts/{alert['alert_id']}")["alert"]
        if detail != alert:
            raise RuntimeError(f"alert detail API disagrees with list API: {detail}")

    def require_storage_is_normalized(self, trace_id: int, path: Path) -> None:
        with sqlite3.connect(self.database) as connection:
            alert_columns = {
                row[1] for row in connection.execute("PRAGMA table_info(alerts)").fetchall()
            }
            expected_columns = {
                "alert_id",
                "trace_id",
                "alert_definition_id",
                "created_at",
                "payload_json",
            }
            if alert_columns != expected_columns:
                raise RuntimeError(f"alerts columns are not minimal: {sorted(alert_columns)}")
            row = connection.execute(
                "SELECT payload_json FROM alerts WHERE trace_id = ?", (trace_id,)
            ).fetchone()
            definition_count = connection.execute(
                "SELECT COUNT(*) FROM alert_definitions WHERE producer_plugin_id = ?",
                ("actrail.file-leakage",),
            ).fetchone()[0]
        if row is None or json.loads(row[0]) != {"residual_files": [str(path)]}:
            raise RuntimeError(f"stored alert payload is invalid: {row}")
        if definition_count != 1:
            raise RuntimeError(f"expected one shared alert definition, found {definition_count}")

    def require_limit_validation(self) -> None:
        self.require_http_status("/api/alerts?limit=0", 400)
        self.require_http_status("/api/alerts?limit=301", 400)

    def populate_latest_window(self, count: int) -> tuple[int, Path]:
        newest: tuple[int, Path] | None = None
        for index in range(count):
            path = self.new_retained_path(f"latest-{index:02}.tmp")
            trace_id = self.launch_retained(f"file-leak-latest-{index:02}", path)
            alert = self.wait_for_trace_alert(trace_id)
            self.require_alert(alert, trace_id, path)
            newest = (trace_id, path)
        if newest is None:
            raise RuntimeError("latest-window population count must be positive")
        return newest

    def get_json(self, path: str) -> dict:
        with self.http.open(f"{self.base_url}{path}", timeout=2.0) as response:
            if response.status != 200:
                raise RuntimeError(f"GET {path} returned HTTP {response.status}")
            return json.loads(response.read().decode("utf-8"))

    def post_json(self, path: str) -> dict:
        request = urllib.request.Request(f"{self.base_url}{path}", method="POST")
        with self.http.open(request, timeout=10.0) as response:
            if response.status != 200:
                raise RuntimeError(f"POST {path} returned HTTP {response.status}")
            return json.loads(response.read().decode("utf-8"))

    def require_http_status(self, path: str, expected: int) -> None:
        try:
            self.http.open(f"{self.base_url}{path}", timeout=2.0)
        except urllib.error.HTTPError as error:
            if error.code == expected:
                return
            raise RuntimeError(f"GET {path} returned HTTP {error.code}, expected {expected}")
        raise RuntimeError(f"GET {path} returned HTTP 200, expected {expected}")

    def require_running(self, process, name: str, log_file) -> None:
        if process is None or process.poll() is not None:
            raise RuntimeError(f"{name} exited early\n{self.read_log(log_file)}")

    @staticmethod
    def read_log(log_file) -> str:
        if log_file is None:
            return ""
        log_file.flush()
        log_file.seek(0)
        return log_file.read()[-8000:]

    def stop_services(self) -> None:
        for process in (self.web, self.daemon):
            if process is None or process.poll() is not None:
                continue
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        for log_file in (self.web_log, self.daemon_log):
            if log_file is not None:
                log_file.close()

    def cleanup(self) -> None:
        for path in self.created_files:
            if path.exists():
                path.unlink()
        if os.environ.get("ACTRAIL_E2E_KEEP_TEMP") != "1" and self.run_dir.exists():
            shutil.rmtree(self.run_dir)


def run_checked(
    command: list[str],
    *,
    cwd: Path,
    timeout: float = 60.0,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(command)}\n{result.stdout[-8000:]}"
        )
    return result


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("file-leakage E2E requires root for eBPF file observation")


def positive_float_env(name: str, default: float) -> float:
    raw = os.environ.get(name)
    value = default if raw is None else float(raw)
    if value <= 0:
        raise RuntimeError(f"{name} must be positive")
    return value


def available_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def parse_fields(raw: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            fields[key.strip()] = value.strip()
    return fields


def shell_quote(path: Path) -> str:
    raw = str(path)
    return "'" + raw.replace("'", "'\"'\"'") + "'"


if __name__ == "__main__":
    try:
        FileLeakageE2E().run()
    except Exception as error:
        print(f"file leakage plugin e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
