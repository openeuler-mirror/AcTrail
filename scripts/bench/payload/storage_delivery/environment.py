"""Owned daemon, online OTLP receiver, and real SQLite failure injection."""

import json
import sqlite3
import tempfile
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import CommonTestConfig, TestOutput
from tests.v2.common.plugin_test_environment import PluginRuntimeSpec, PluginTestEnvironment
from tests.v2.regression.otel_http.receiver import OtlpHttpReceiver


class DeliveryEnvironment(PluginTestEnvironment):
    def __init__(self, root: Path, bins: Path, work: Path, scenario: str):
        config = CommonTestConfig(root, bins, work, 30, 60, 150, 0.2)
        self.scenario = scenario
        self.receiver = OtlpHttpReceiver()
        self.database = work / "data/actrail.sqlite"
        self.patch = work / "actraild.patch.toml"
        self.socket_directory = tempfile.TemporaryDirectory(prefix="actrail-sd-")
        super().__init__(config, TestOutput(), operator_config=work / "actraild.conf",
            operator_config_patch=self.patch, web_host="127.0.0.1", web_port=0,
            plugin=PluginRuntimeSpec("otel-http", "storage-delivery", "otel-http", "builtin"))

    def prepare(self):
        self.config.work_dir.mkdir(parents=True)
        ActrailRuntime.write_isolated_operator_config_patch(self.patch, self.config.work_dir,
            plugin_directory=self.config.repo / "examples/plugins/builtin")
        document = self.patch.read_text()
        for name in ("control.sock", "tls-sync.sock"):
            document = document.replace(str(self.config.work_dir / "run" / name),
                                        str(Path(self.socket_directory.name) / name))
        self.patch.write_text(document)
        if self.scenario == "agent-identity":
            with self.patch.open("a") as patch:
                patch.write("\n[capture]\nagent_descendant_observation_depth = 0\n")
        self.receiver.start()
        super().prepare()
        candidate = self.current_config()
        kinds = {"agent.identity", "llm.request", "llm.response", "command.invocation"}
        candidate["action_kinds"] = {key: key in kinds for key in candidate["action_kinds"]}
        candidate.update(endpoint=self.receiver.endpoint, allow_insecure=True,
            encoding="json", compression="none", attribute_mode="metadata-only",
            queue_capacity=128, batch_max_spans=128, batch_timeout_ms=100,
            connect_timeout_ms=250, request_timeout_ms=1000, retry_max_attempts=1,
            retry_backoff_ms=1, shutdown_flush_deadline_ms=3000, headers=[])
        self.update_config(candidate)

    def install_fault(self, previous_trace: int):
        if self.scenario == "events":
            table, condition = "events", "1"
        else:
            table, condition = "semantic_actions", "NEW.kind_code = 119"
        # Identifiers and predicates above are fixture constants, never user SQL.
        sql = (f"CREATE TRIGGER acceptance_storage_failure BEFORE INSERT ON {table} "
               f"WHEN NEW.trace_id > {int(previous_trace)} AND ({condition}) "
               "BEGIN SELECT RAISE(ABORT, 'acceptance:storage_delivery_failure'); END;")
        (self.config.work_dir / "fault.sql").write_text(sql + "\n")
        with sqlite3.connect(self.database, timeout=5) as database:
            database.execute(sql)

    def spans(self):
        return [span for document in self.receiver.documents()
                for resource in document.get("resourceSpans", [])
                for scope in resource.get("scopeSpans", []) for span in scope.get("spans", [])]

    @staticmethod
    def attributes(span):
        return {item["key"]: next(iter(item["value"].values()))
                for item in span.get("attributes", []) if item.get("value")}

    def close(self):
        failures = []
        try:
            if self._plugin_loaded:
                try:
                    self.unload_plugin()
                except Exception as error:
                    failures.append(f"unload own plugin: {error}")
            self._stop_web(failures)
            owned_pid = self.config.work_dir / "run/actraild.pid"
            if owned_pid.exists():
                pid = int(owned_pid.read_text())
                process = Path(f"/proc/{pid}")
                try:
                    executable = (process / "exe").resolve(strict=True)
                    arguments = (process / "cmdline").read_bytes().split(b"\0")
                    own_config = str(self.operator_config).encode()
                    if executable != self.config.bin_dir / "actraild" or not any(
                        key == b"--config" and value == own_config
                        for key, value in zip(arguments, arguments[1:])
                    ):
                        raise RuntimeError("refusing to stop process not owned by this fixture")
                except FileNotFoundError:
                    process = None
            else:
                process = None
            if process is not None:
                stopped = self.runtime.stop()
                if stopped is None or stopped.returncode:
                    failures.append("stop own daemon failed")
        finally:
            if self.config.work_dir.exists():
                (self.config.work_dir / "online-otlp.json").write_text(
                    json.dumps(self.receiver.documents(), indent=2) + "\n")
            self.receiver.stop()
        if failures:
            raise RuntimeError("; ".join(failures))
        self.socket_directory.cleanup()
