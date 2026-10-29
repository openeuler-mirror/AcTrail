"""Exercise the resource HTTP readers against completed real cgroup traces."""
from __future__ import annotations

from collections import defaultdict
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, build_opener

from tests.v2.common.core import TestCaseInputs
from .constants import CASE_CONSTANTS as C


@contextmanager
def _web(inputs: TestCaseInputs, config: Path):
    binary = inputs.bin_dir / "actrailweb"
    if not binary.is_absolute():
        binary = inputs.repo / binary
    log_path = inputs.work_dir / "resource-api-web.log"
    with log_path.open("w", encoding="utf-8") as log:
        process = subprocess.Popen(
            [str(binary), "--config", str(config), "--addr", "127.0.0.1", "--port", "0"],
            cwd=inputs.repo, stdout=log, stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + C.web_ready_timeout
            while time.monotonic() < deadline:
                text = log_path.read_text(encoding="utf-8")
                if process.poll() is not None:
                    raise RuntimeError(f"resource API web exited: {text[-2000:]}")
                address = next((line.removeprefix("actrailweb listening on ").split()[0]
                                for line in text.splitlines() if line.startswith("actrailweb listening on ")), None)
                if address:
                    yield address
                    return
                time.sleep(C.web_poll_seconds)
            raise RuntimeError("resource API web startup timed out")
        finally:
            process.terminate()
            try:
                process.wait(timeout=C.web_stop_timeout)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=C.web_stop_timeout)


def _nanos(sample: dict) -> int:
    observed = sample["observed_at"]
    return observed["secs_since_epoch"] * 1_000_000_000 + observed["nanos_since_epoch"]


def _assert_timeline(timeline: dict, trace_id: int, start: int, end: int, samples: list[dict]) -> None:
    if (timeline["trace_id"], timeline["from_ms"], timeline["to_ms"]) != (trace_id, start, end):
        raise AssertionError("resource timeline returned another trace or time window")
    selected = [sample for sample in samples if start * 1_000_000 <= _nanos(sample) <= end * 1_000_000]
    points = timeline["workload"]["points"]
    if sum(point["samples"] for point in points) != len(selected):
        raise AssertionError("resource timeline sample count differs from stored samples in its window")
    if bool(selected) != timeline["workload"]["available"]:
        raise AssertionError("resource timeline availability differs from stored samples")
    if not selected:
        if points or "no_samples" not in timeline["workload"]["reasons"]:
            raise AssertionError("empty resource window did not report no_samples")
        return
    if timeline["workload"]["source"] != "trace":
        raise AssertionError("managed workload timeline lost its trace source")
    buckets = defaultdict(list)
    for sample in selected:
        bucket = start + ((_nanos(sample) // 1_000_000 - start) // timeline["bucket_ms"]) * timeline["bucket_ms"]
        buckets[bucket].append(sample["payload"])
    if [point["t_ms"] for point in points] != sorted(buckets):
        raise AssertionError("resource timeline buckets are missing, duplicated or unordered")
    for point in points:
        payloads = buckets[point["t_ms"]]
        memory = [payload["memory_current_bytes"] for payload in payloads if payload["memory_current_bytes"] is not None]
        pids = [payload["pids_current"] for payload in payloads if payload["pids_current"] is not None]
        if point["memory_bytes_max"] != max(memory, default=None) or point["pids_max"] != max(pids, default=None):
            raise AssertionError("resource timeline memory/PID peaks differ from stored samples")


def assert_resource_api(inputs: TestCaseInputs, config: Path, traces: dict[int, list[dict]]) -> str:
    opener = build_opener(ProxyHandler({}))
    with _web(inputs, config) as base:
        def read(path: str) -> dict:
            with opener.open(base + path, timeout=C.web_request_timeout) as response:
                return json.load(response)
        for trace_id, samples in traces.items():
            if not samples:
                raise AssertionError(f"trace-{trace_id} has no resource API reference samples")
            path = f"/api/traces/{trace_id}"
            if read(path + f"/resources?limit={C.limit}")["resources"] != samples:
                raise AssertionError(f"trace-{trace_id} resource API differs from complete viewer samples")
            if read(path + "/resources?limit=1")["resources"] != samples[:1]:
                raise AssertionError("resource API did not preserve newest-first limit")
            times = sorted(_nanos(sample) // 1_000_000 for sample in samples)
            windows = ((times[0] - 1, times[-1] + 2),
                       (times[len(times) // 2], times[-1] + 2),
                       (times[0] - 2000, times[0] - 1000))
            for start, end in windows:
                timeline = read(path + f"/resource-timeline?from_ms={start}&to_ms={end}&max_points={C.resource_api_max_points}")
                _assert_timeline(timeline, trace_id, start, end, samples)
            for suffix in ("/resources?limit=0", "/resource-timeline?from_ms=2&to_ms=1", "/resource-timeline?max_points=0"):
                try:
                    read(path + suffix)
                except HTTPError as error:
                    if error.code != 400:
                        raise
                else:
                    raise AssertionError(f"invalid resource query was accepted: {suffix}")
    return f"{len(traces)} traces: complete samples, limits, windowed bucket peaks, empty windows and invalid parameters verified"
