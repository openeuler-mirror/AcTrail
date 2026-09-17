#!/usr/bin/env python3
"""Compare current AcTrail cgroup-v2 measurements with RSS and PSS.

The historical experiment attached to an existing process tree and therefore
measured AcTrail's procfs RSS aggregation.  The current cgroup implementation
requires a controlled launch, so this reproduction starts actraild in a
delegated transient systemd service and runs the corpus workload through
``actrailctl launch``.
"""

import argparse
import hashlib
import json
import os
import platform
import re
import sqlite3
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


DEFAULT_RESOURCE_ORACLE = Path("/home/projects/resource-oracle")


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--resource-oracle", type=Path, default=DEFAULT_RESOURCE_ORACLE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--seconds", type=float, default=12.0)
    parser.add_argument("--workers", type=int, default=4)
    return parser.parse_args()


def command(argv, *, cwd=None, timeout=30):
    return subprocess.run(
        argv,
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def wait_until(condition, message, *, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(0.05)
    raise TimeoutError(message)


def read_os_release():
    values = {}
    with open("/etc/os-release", encoding="utf-8") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if line and not line.startswith("#") and "=" in line:
                key, value = line.split("=", 1)
                values[key] = value.strip('"')
    return values


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_int(path):
    return int(path.read_text(encoding="ascii"))


def read_pss_kib(pid):
    with open(f"/proc/{pid}/smaps_rollup", encoding="ascii") as handle:
        for line in handle:
            if line.startswith("Pss:"):
                return int(line.split()[1])
    raise RuntimeError(f"Pss missing for pid {pid}")


def read_rss_kib(pid):
    with open(f"/proc/{pid}/statm", encoding="ascii") as handle:
        resident_pages = int(handle.read().split()[1])
    return resident_pages * os.sysconf("SC_PAGE_SIZE") // 1024


def subtree_pids(cgroup):
    pids = set()
    for procs in cgroup.rglob("cgroup.procs"):
        try:
            pids.update(int(line) for line in procs.read_text().splitlines() if line)
        except FileNotFoundError:
            pass
    return sorted(pids)


def sample_external(cgroup):
    sampled = []
    pss_kib = 0
    rss_kib = 0
    for pid in subtree_pids(cgroup):
        try:
            process_pss_kib = read_pss_kib(pid)
            process_rss_kib = read_rss_kib(pid)
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        pss_kib += process_pss_kib
        rss_kib += process_rss_kib
        sampled.append(pid)
    return {
        "record": "external_sample",
        "time_unix_ns": time.time_ns(),
        "pids": sampled,
        "process_count": len(sampled),
        "statm_rss_sum_kib": rss_kib,
        "pss_kib": pss_kib,
        "cgroup_memory_current_kib": read_int(cgroup / "memory.current") // 1024,
        "cgroup_memory_peak_kib": read_int(cgroup / "memory.peak") // 1024,
    }


def trace_cgroup(service_cgroup):
    traces = service_cgroup / "traces"
    if not traces.is_dir():
        return None
    candidates = [path for path in traces.iterdir() if path.name.startswith("trace-")]
    return candidates[0] if len(candidates) == 1 else None


def trace_id_from_cgroup(cgroup):
    match = re.fullmatch(r"trace-(\d+)-[0-9a-f]+", cgroup.name)
    if not match:
        raise RuntimeError(f"unexpected trace cgroup name: {cgroup.name}")
    return int(match.group(1))


def write_patch(path, run_dir, service_cgroup):
    path.write_text(
        f'''[control]
socket_path = "{run_dir / 'control.sock'}"
pid_file = "{run_dir / 'actraild.pid'}"
log_path = "{run_dir / 'actraild.log'}"

[control.finalization]
poll_interval_ms = 25
settle_delay_ms = 25

[storage.sqlite]
path = "{run_dir / 'actrail.sqlite'}"

[storage.retention]
enabled = false

[export.snapshot]
directory = "{run_dir / 'export'}"

[plugins.discovery]
directory = "{run_dir / 'plugins'}"

[plugins.startup]
enabled = false
load = []

[hand_observation]
enabled = false

[sandbox_alerts]
enabled = false

[capture]
profile_name = "resource-memory-reproduction"
capabilities = ["resource-metrics"]
opportunistic_capabilities = []
disabled_capabilities = []

[ebpf]
enabled = "false"

[payload.tls]
enabled = false
sync_event_socket_path = "{run_dir / 'tls-sync.sock'}"

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

[file_observation]
enabled = false

[application]
enabled = false
http1_enabled = false
http2_enabled = false

[resource_metrics]
enabled = true
mode = "cgroup-v2"
interval_ms = 250
cgroup_root = "{service_cgroup}"
finalization_timeout_ms = 5000
orphan_limit = 64
memory_alert_rss_kb = "1"

[enforcement]
enabled = false
''',
        encoding="utf-8",
    )


def resource_payloads(viewer, database, trace_id, repo_root):
    result = command(
        [
            str(viewer),
            "--storage-path",
            str(database),
            "--output-format",
            "json",
            "events",
            "--trace-id",
            f"trace-{trace_id}",
        ],
        cwd=repo_root,
    )
    events = json.loads(result.stdout)["events"]
    return [event["payload"] for event in events if event["variant"] == "resource"]


def median_int(rows, key):
    return int(statistics.median(row[key] for row in rows))


def run(args):
    if os.geteuid() != 0:
        raise RuntimeError("run as root for delegated cgroup setup")
    if args.seconds <= 0 or args.workers <= 0:
        raise RuntimeError("seconds and workers must be positive")
    repo_root = Path(__file__).resolve().parents[3]
    workload = Path(__file__).with_name("swebench_corpus_workload.py")
    binaries = repo_root / "target/release"
    actraild = binaries / "actraild"
    actrailctl = binaries / "actrailctl"
    viewer = binaries / "actrailviewer"
    python = args.resource_oracle / ".venv/bin/python"
    corpus = args.resource_oracle / "data_corpus/corpus_f2b_swe.jsonl"
    parquet = (
        args.resource_oracle
        / ".corpus_scenarios_large/swe_bench_verified/swe-bench_verified.parquet"
    )
    required = [actraild, actrailctl, viewer, python, corpus, parquet, workload]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise RuntimeError("required files are missing: " + ", ".join(missing))
    if not Path("/run/systemd/system").is_dir():
        raise RuntimeError("systemd is not the active service manager")

    unit = f"actrail-memory-reproduction-{os.getpid()}"
    service = f"{unit}.service"
    service_cgroup = Path("/sys/fs/cgroup/system.slice") / service
    records = []
    launch = None
    try:
        with tempfile.TemporaryDirectory(prefix="actrail-memory-reproduction.") as raw:
            run_dir = Path(raw)
            for name in ("export", "plugins"):
                (run_dir / name).mkdir()
            config = run_dir / "operator.conf"
            patch = run_dir / "operator.patch.toml"
            database = run_dir / "actrail.sqlite"
            ready = run_dir / "workload.ready"
            start = run_dir / "workload.start"
            launch_stdout = run_dir / "launch.stdout"
            launch_stderr = run_dir / "launch.stderr"
            write_patch(patch, run_dir, service_cgroup)
            command(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "init",
                    "--force",
                    "--patch",
                    str(patch),
                ],
                cwd=repo_root,
            )
            command(
                [
                    "systemd-run",
                    f"--unit={unit}",
                    "--property=Delegate=yes",
                    "--property=DelegateSubgroup=daemon",
                    "--property=Type=exec",
                    "--property=KillMode=process",
                    f"--property=WorkingDirectory={repo_root}",
                    str(actraild),
                    "--config",
                    str(config),
                    "run",
                ],
                cwd=repo_root,
            )
            wait_until(
                lambda: (run_dir / "control.sock").exists(),
                "daemon control socket was not created",
            )

            workload_command = [
                str(python),
                str(workload),
                "--corpus",
                str(corpus),
                "--parquet",
                str(parquet),
                "--ready-file",
                str(ready),
                "--start-file",
                str(start),
                "--seconds",
                str(args.seconds),
                "--workers",
                str(args.workers),
            ]
            with open(launch_stdout, "w", encoding="utf-8") as stdout, open(
                launch_stderr, "w", encoding="utf-8"
            ) as stderr:
                launch = subprocess.Popen(
                    [
                        str(actrailctl),
                        "launch",
                        "--config",
                        str(config),
                        "--name",
                        "swebench-corpus-analytics",
                        "--host-ebpf",
                        "disabled",
                        "--seccomp-notify",
                        "disabled",
                        "--",
                        *workload_command,
                    ],
                    cwd=repo_root,
                    stdout=stdout,
                    stderr=stderr,
                )
                wait_until(lambda: ready.exists(), "workload workers did not become ready")
                cgroup = wait_until(
                    lambda: trace_cgroup(service_cgroup),
                    "trace cgroup was not created",
                )
                trace_id = trace_id_from_cgroup(cgroup)
                start.touch()
                while launch.poll() is None:
                    records.append(sample_external(cgroup))
                    time.sleep(0.1)
                if launch.wait() != 0:
                    raise RuntimeError(
                        f"launch failed: {launch_stderr.read_text(encoding='utf-8')}"
                    )
                launch = None

            wait_until(
                lambda: sqlite_trace_finalized(database, trace_id),
                "trace did not finalize",
                timeout=15,
            )
            payloads = resource_payloads(viewer, database, trace_id, repo_root)
            expected_processes = args.workers + 1
            external_plateau = [
                row for row in records if row["process_count"] == expected_processes
            ]
            actrail_plateau = [
                row
                for row in payloads
                if row["sample_kind"] == "periodic"
                and int(
                    row.get("metadata", {}).get(
                        "process_rss_sampled_processes", "0"
                    )
                )
                == expected_processes
            ]
            if not external_plateau or not actrail_plateau:
                raise RuntimeError(
                    "missing plateau samples: "
                    f"external={len(external_plateau)} AcTrail={len(actrail_plateau)}"
                )
            if any(row["accounting_method"] != "cgroup_v2" for row in payloads):
                raise RuntimeError("AcTrail unexpectedly fell back from cgroup-v2")
            if any(row["accounting_coverage"] != "exact" for row in payloads):
                raise RuntimeError("AcTrail emitted non-exact cgroup coverage")

            external_rss = median_int(external_plateau, "statm_rss_sum_kib")
            external_pss = median_int(external_plateau, "pss_kib")
            external_cgroup = median_int(external_plateau, "cgroup_memory_current_kib")
            actrail_cgroup = median_int(actrail_plateau, "memory_current_bytes") // 1024
            actrail_rss = median_int(actrail_plateau, "process_rss_sum_kb")
            environment = {
                "record": "environment",
                "observed_date": time.strftime("%Y-%m-%d", time.localtime()),
                "actrail_revision": command(
                    ["git", "rev-parse", "--short", "HEAD"], cwd=repo_root
                ).stdout.strip(),
                "os_pretty_name": read_os_release().get("PRETTY_NAME"),
                "kernel_release": platform.release(),
                "architecture": platform.machine(),
                "python": command([str(python), "--version"]).stdout.strip(),
                "cgroup_version": 2,
                "corpus_path": str(corpus),
                "corpus_sha256": sha256_file(corpus),
                "parquet_path": str(parquet),
                "parquet_sha256": sha256_file(parquet),
                "workers": args.workers,
                "seconds": args.seconds,
                "launch_mode": "controlled actrailctl launch",
            }
            summary = {
                "record": "summary",
                "expected_processes": expected_processes,
                "external_plateau_samples": len(external_plateau),
                "actrail_plateau_samples": len(actrail_plateau),
                "actrail_periodic_samples": sum(
                    row["sample_kind"] == "periodic" for row in payloads
                ),
                "actrail_final_samples": sum(
                    row["sample_kind"] == "final" for row in payloads
                ),
                "accounting_method": "cgroup_v2",
                "accounting_coverage": sorted(
                    {row["accounting_coverage"] for row in actrail_plateau}
                ),
                "actrail_memory_current_median_kib": actrail_cgroup,
                "external_cgroup_memory_current_median_kib": external_cgroup,
                "actrail_process_rss_sum_median_kib": actrail_rss,
                "external_statm_rss_sum_median_kib": external_rss,
                "external_pss_median_kib": external_pss,
                "external_cgroup_memory_peak_kib": max(
                    row["cgroup_memory_peak_kib"] for row in external_plateau
                ),
                "actrail_to_external_cgroup_ratio": actrail_cgroup / external_cgroup,
                "actrail_rss_to_external_rss_ratio": actrail_rss / external_rss,
                "rss_to_cgroup_current_ratio": actrail_rss / actrail_cgroup,
                "rss_to_pss_ratio": actrail_rss / external_pss,
            }
            return [environment, *records, *payload_records(payloads), summary]
    finally:
        if launch is not None and launch.poll() is None:
            launch.terminate()
        # Kill only this PID-scoped transient unit so a failed reproduction
        # cannot leave a corpus worker behind in its delegated trace subtree.
        subprocess.run(
            ["systemctl", "kill", "--kill-whom=all", "--signal=SIGKILL", service],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if launch is not None and launch.poll() is None:
            try:
                launch.wait(timeout=10)
            except subprocess.TimeoutExpired:
                launch.kill()
                launch.wait(timeout=10)
        subprocess.run(
            ["systemctl", "stop", service],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ["systemctl", "reset-failed", service],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


def sqlite_trace_finalized(database, trace_id):
    if not database.is_file():
        return False
    try:
        with sqlite3.connect(database) as connection:
            row = connection.execute(
                "SELECT traces.lifecycle_state, trace_resource_scopes.lifecycle_state "
                "FROM traces JOIN trace_resource_scopes USING(trace_id) WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
    except sqlite3.OperationalError:
        return False
    return row == ("completed", "finalized")


def payload_records(payloads):
    for payload in payloads:
        yield {"record": "actrail_resource_sample", **payload}


def main():
    args = parse_args()
    records = run(args)
    rendered = "".join(json.dumps(record, sort_keys=True) + "\n" for record in records)
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")


if __name__ == "__main__":
    main()
