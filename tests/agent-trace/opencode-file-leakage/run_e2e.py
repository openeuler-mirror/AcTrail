#!/usr/bin/env python3
"""Verify a real OpenCode trace reports its leaked OpenTUI shared object."""

from __future__ import annotations

import importlib.util
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HARNESS_PATH = ROOT / "tests/plugins/file-leakage/run_e2e.py"
RUN_DIR = ROOT / "temp/opencode-file-leakage-e2e"
TMP_DIR = Path("/tmp")
ARTIFACT_NAME = re.compile(r"^\.[0-9a-f]+-00000000\.so$")
DEFAULT_OPENCODE_VERSION = "1.15.11"
DEFAULT_DEADLINE_SECONDS = 120.0
PROMPT = "只回复 OPENCODE_FILE_LEAK_E2E；不要读取、创建或修改任何文件。"
EXPECTED_OUTPUT = "OPENCODE_FILE_LEAK_E2E"


def load_harness():
    spec = importlib.util.spec_from_file_location("actrail_file_leakage_harness", HARNESS_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load file-leakage harness {HARNESS_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


HARNESS = load_harness()


class OpenCodeFileLeakageE2E(HARNESS.FileLeakageE2E):
    def __init__(self) -> None:
        super().__init__(RUN_DIR)
        configured = os.environ.get("ACTRAIL_E2E_OPENCODE_BIN")
        entry = configured or shutil.which("opencode")
        if entry is None:
            raise RuntimeError(
                "opencode is not on PATH; set ACTRAIL_E2E_OPENCODE_BIN explicitly"
            )
        self.opencode = Path(entry).resolve()
        if not self.opencode.is_file() or not os.access(self.opencode, os.X_OK):
            raise RuntimeError(f"configured OpenCode is not executable: {self.opencode}")
        self.expected_version = os.environ.get(
            "ACTRAIL_E2E_OPENCODE_VERSION", DEFAULT_OPENCODE_VERSION
        )
        self.alert_deadline_seconds = HARNESS.positive_float_env(
            "ACTRAIL_E2E_ALERT_DEADLINE_SECONDS", DEFAULT_DEADLINE_SECONDS
        )
        self.launch_timeout_seconds = HARNESS.positive_float_env(
            "ACTRAIL_E2E_OPENCODE_LAUNCH_TIMEOUT_SECONDS", DEFAULT_DEADLINE_SECONDS
        )

    def run(self) -> None:
        HARNESS.require_root()
        version = self.require_version()
        before = self.artifacts()
        self.prepare_runtime()
        try:
            self.start_services()
            self.require_catalog_lifecycle_and_load()
            trace_id, output = self.launch_with_output(
                "opencode-file-leakage",
                [
                    str(self.opencode),
                    "run",
                    "--pure",
                    "--format",
                    "json",
                    "--title",
                    "actrail-file-leak-e2e",
                    PROMPT,
                ],
            )
            artifact = self.wait_for_artifact(before)
            self.created_files.append(artifact)
            self.require_opentui_artifact(artifact)
            if EXPECTED_OUTPUT not in output:
                raise RuntimeError("OpenCode output did not contain the expected response marker")

            alert = self.wait_for_trace_alert(trace_id)
            self.require_opencode_alert(alert, trace_id, artifact)
            self.require_trace_evicted(trace_id)
            self.require_persisted_source_path(trace_id, artifact)
            self.require_trace_api_matches(alert)
            self.require_detail_api_matches(alert)
            latest = self.get_json("/api/alerts")["alerts"]
            if not any(row.get("alert_id") == alert.get("alert_id") for row in latest):
                raise RuntimeError("OpenCode alert is absent from the global latest alerts API")
            self.require_web_unload()

            print(f"opencode_version={version}")
            print(f"opencode_file_leakage_trace_id={trace_id}")
            print(f"opencode_file_leakage_alert_id={alert['alert_id']}")
            print(f"opencode_file_leakage_artifact={artifact}")
            print("opencode file leakage e2e complete")
        finally:
            self.stop_services()
            self.cleanup()

    def require_version(self) -> str:
        result = subprocess.run(
            [str(self.opencode), "--version"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=10,
        )
        if result.returncode != 0:
            raise RuntimeError(f"OpenCode version check failed:\n{result.stdout[-4000:]}")
        version = result.stdout.strip()
        if version != self.expected_version:
            raise RuntimeError(
                f"OpenCode version is {version!r}, expected {self.expected_version!r}"
            )
        return version

    def artifacts(self) -> set[Path]:
        return {
            entry.resolve()
            for entry in TMP_DIR.iterdir()
            if ARTIFACT_NAME.fullmatch(entry.name) and entry.is_file()
        }

    def wait_for_artifact(self, before: set[Path]) -> Path:
        deadline = time.monotonic() + self.alert_deadline_seconds
        while time.monotonic() < deadline:
            created = self.artifacts() - before
            if len(created) == 1:
                return next(iter(created))
            if len(created) > 1:
                raise RuntimeError(
                    "OpenCode created multiple matching hidden shared objects: "
                    + ", ".join(str(path) for path in sorted(created))
                )
            time.sleep(0.05)
        raise RuntimeError("OpenCode did not create a new /tmp/.<hex>-00000000.so artifact")

    @staticmethod
    def require_opentui_artifact(path: Path) -> None:
        content = path.read_bytes()
        if not content.startswith(b"\x7fELF"):
            raise RuntimeError(f"OpenCode artifact is not an ELF file: {path}")
        if b"libopentui.so" not in content:
            raise RuntimeError(f"OpenCode artifact lacks the libopentui.so marker: {path}")

    @staticmethod
    def require_opencode_alert(alert: dict, trace_id: int, artifact: Path) -> None:
        expected = {
            "trace_id": trace_id,
            "producer_plugin_id": "actrail.file-leakage",
            "definition_key": "file-leakage",
            "kind": "file.leakage",
            "title": "存在文件泄露",
            "severity": "medium",
        }
        for field, value in expected.items():
            if alert.get(field) != value:
                raise RuntimeError(
                    f"alert field {field}={alert.get(field)!r}, expected {value!r}"
                )
        residual_files = alert.get("payload", {}).get("residual_files")
        if not isinstance(residual_files, list) or str(artifact) not in residual_files:
            raise RuntimeError(
                f"OpenCode artifact is absent from alert residual_files: {residual_files!r}"
            )
        forbidden = {"config", "metadata", "evidence", "actions", "alert_definition_id"}
        present = forbidden.intersection(alert)
        if present:
            raise RuntimeError(f"alert API leaked duplicate fields: {sorted(present)}")


if __name__ == "__main__":
    try:
        OpenCodeFileLeakageE2E().run()
    except Exception as error:
        print(f"opencode file leakage e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
