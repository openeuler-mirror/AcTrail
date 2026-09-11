from __future__ import annotations

import os
import sqlite3
import subprocess
from pathlib import Path

from tests.v2.regression.resource_metrics_cgroup.case import ResourceMetricsCgroupCase


def assert_container_samples(payloads: list[dict], container_id: str, readings: list[int], *, expect_rss: bool = False) -> None:
    periodic = [p for p in payloads if p["sample_kind"] == "periodic"]
    if not periodic:
        raise AssertionError("no periodic container samples")
    # A sleeping workload is nearly stable, but counter reads are not atomic
    # with AcTrail's sampling. Allow 4 MiB of kernel accounting jitter.
    low, high = min(readings) - 4 * 1024**2, max(readings) + 4 * 1024**2
    for payload in periodic:
        if (payload["scope"], payload["accounting_method"], payload["accounting_coverage"]) != (
            "container", "cgroup_v2", "broader_than_trace"
        ):
            raise AssertionError(f"incorrect container semantics: {payload}")
        if payload["subject"] != f"container:{container_id}":
            raise AssertionError("sample refers to another container")
        if not low <= payload["memory_current_bytes"] <= high:
            raise AssertionError("container sample disagrees with direct kernel readings")
        if expect_rss:
            if not payload.get("process_rss_sum_kb") or payload.get("rss_kb") != payload["process_rss_sum_kb"]:
                raise AssertionError("configured RSS alert requires explicit process RSS")
            if str(payload.get("metadata", {}).get("alert")).lower() != "true":
                raise AssertionError("RSS-only threshold did not trigger an alert")
        elif payload.get("rss_kb") is not None:
            raise AssertionError("charged memory must not be labelled RSS")


class ContainerAcceptance(ResourceMetricsCgroupCase):
    """Standalone acceptance using an isolated daemon and disposable Docker container."""

    def _operator_patch(self) -> str:
        patch = super()._operator_patch().replace('mode = "cgroup-v2"', 'mode = "auto"')
        patch = patch.replace(f'cgroup_root = "{self._cgroup}"',
                              f'cgroup_root = "{self._inputs.work_dir / "unavailable-managed-root"}"')
        return patch.replace('memory_alert_rss_kb = "1"',
                             'existing_container_cgroups = "require"\nmemory_alert_rss_kb = "1"')

    def exercise(self, image: str) -> None:
        self._prepare_config()
        container_id = ""
        daemon = None
        log = (self._inputs.work_dir / "daemon-output.log").open("w")
        try:
            container_id = self._command(("docker", "run", "--pull=never", "-d", "--network=none",
                                          "--entrypoint", "/bin/sh", image, "-c", "exec sleep 120")).stdout.strip()
            if len(container_id) != 64 or any(c not in "0123456789abcdef" for c in container_id):
                raise AssertionError("docker did not return a full container ID")
            pid = int(self._command(("docker", "inspect", "--format", "{{.State.Pid}}", container_id)).stdout)
            membership = Path(f"/proc/{pid}/cgroup").read_text()
            daemon = self._start_isolated_daemon(log)
            self._command((str(self._binary("actrailctl")), "track-add", "--config", str(self._config),
                           "--pid", str(pid), "--name", "container-cgroup-acceptance"))
            self._wait_until(lambda: self._event_count(1) >= 3, "no initial samples")
            with sqlite3.connect(self._database) as connection:
                row = connection.execute("SELECT relative_path FROM trace_external_cgroup_bindings WHERE trace_id=1").fetchone()
            if row is None:
                raise AssertionError("container attachment did not persist a binding")
            boundary = Path("/sys/fs/cgroup") / str(row[0]).lstrip("/")
            before = self._controls(boundary)
            readings = [int((boundary / "memory.current").read_text())]
            count = self._event_count(1)
            daemon.kill()
            daemon.wait(timeout=10)
            # Startup recovers the durable external binding without reattaching.
            daemon = self._start_isolated_daemon(log)
            self._wait_until(lambda: self._event_count(1) >= count + 3, "sampling did not recover")
            readings.append(int((boundary / "memory.current").read_text()))
            assert_container_samples(self._resource_payloads(1), container_id, readings, expect_rss=True)
            if self._controls(boundary) != before or Path(f"/proc/{pid}/cgroup").read_text() != membership:
                raise AssertionError("read-only sampling changed container controls or membership")
            # Also retain live explicit-removal coverage while the workload runs.
            self._command((str(self._binary("actrailctl")), "track-add", "--config", str(self._config),
                           "--pid", str(pid), "--name", "container-cgroup-final"))
            self._wait_until(lambda: self._event_count(2) >= 2, "second trace has no samples")
            self._command((str(self._binary("actrailctl")), "track-remove", "--config", str(self._config),
                           "--trace-id", "trace-2"))
            def closed() -> bool:
                with sqlite3.connect(self._database) as connection:
                    return connection.execute("SELECT lifecycle_state FROM trace_external_cgroup_bindings WHERE trace_id=2").fetchone() == ("closed",)
            self._wait_until(closed, "external finalization did not close binding")
            self._assert_final(self._resource_payloads(2), coverage="broader_than_trace")
            self._command(("docker", "stop", "--time", "1", container_id))
            def recovered_closed() -> bool:
                with sqlite3.connect(self._database) as connection:
                    return connection.execute(
                        "SELECT b.lifecycle_state, t.lifecycle_state FROM trace_external_cgroup_bindings b "
                        "JOIN traces t USING(trace_id) WHERE trace_id=1"
                    ).fetchone() == ("closed", "completed")
            self._wait_until(recovered_closed, "original recovered trace did not complete after process exit")
            finals = [p for p in self._resource_payloads(1) if p["sample_kind"] == "final"]
            if len(finals) != 1:
                raise AssertionError("original recovered trace must have exactly one final sample")
            daemon.kill()
            daemon.wait(timeout=10)
            daemon = self._start_isolated_daemon(log)
            if len([p for p in self._resource_payloads(1) if p["sample_kind"] == "final"]) != 1:
                raise AssertionError("restart duplicated recovered final sample")
        finally:
            if daemon is not None and daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait(timeout=10)
            log.close()
            if container_id:
                self._command(("docker", "rm", "-f", container_id))

    def _start_isolated_daemon(self, log):
        process = subprocess.Popen((str(self._binary("actraild")), "--config", str(self._config), "run"),
                                   cwd=self._inputs.repo, stdout=log, stderr=log)
        try:
            self._wait_until(lambda: process.poll() is None and self._doctor_ok(), "daemon not ready")
        except Exception:
            process.terminate()
            process.wait(timeout=10)
            raise
        return process

    @staticmethod
    def _controls(boundary: Path) -> dict[str, str]:
        return {name: (boundary / name).read_text() for name in
                ("memory.max", "memory.high", "cpu.max", "cgroup.subtree_control", "cgroup.procs")
                if (boundary / name).exists()}
