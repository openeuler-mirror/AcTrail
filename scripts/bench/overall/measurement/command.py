"""Command execution and resource measurement."""

from __future__ import annotations

import os
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Sequence

from .proc_tree_sampler import ProcTreeSampler
from .sample import Sample


def measure_command(
    command: Sequence[str],
    *,
    cwd: Path,
    extra_pids: Sequence[int] = (),
    extra_baselines: dict[int, float] | None = None,
    timeout_seconds: float = 900.0,
) -> Sample:
    started = time.perf_counter()
    process = subprocess.Popen(
        list(command),
        cwd=str(cwd),
        env={
            **os.environ,
            "PWD": str(cwd),
        },
        stdout=subprocess.DEVNULL,
        stderr=tempfile.TemporaryFile(mode="w+t", encoding="utf-8"),
        text=True,
    )
    stderr_file = process.stderr
    sampler = ProcTreeSampler(process.pid)
    extra_samplers = [ProcTreeSampler(pid) for pid in extra_pids]
    peak_rss = 0
    extra_peak_rss = 0
    cpu_seconds = 0.0
    while True:
        if process.poll() is not None:
            break
        sampled_cpu, rss_kb, _ = sampler.sample()
        cpu_seconds = max(cpu_seconds, sampled_cpu)
        peak_rss = max(peak_rss, rss_kb)
        for extra in extra_samplers:
            _, extra_rss, _ = extra.sample()
            extra_peak_rss = max(extra_peak_rss, extra_rss)
        if time.perf_counter() - started > timeout_seconds:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
            stderr = ""
            if stderr_file is not None:
                stderr_file.seek(0)
                stderr = stderr_file.read()[-3000:]
                stderr_file.close()
            raise RuntimeError(
                f"command timed out after {timeout_seconds}s: "
                + " ".join(command)
                + "\n"
                + stderr.strip()[-3000:]
            )
        time.sleep(0.05)
    final_cpu, final_rss, _ = sampler.sample()
    cpu_seconds = max(cpu_seconds, final_cpu)
    peak_rss = max(peak_rss, final_rss)
    extra_cpu = 0.0
    for extra, pid in zip(extra_samplers, extra_pids):
        extra_cpu_seconds, extra_rss, _ = extra.sample()
        baseline_cpu = (
            extra_baselines.get(pid, 0.0)
            if extra_baselines is not None
            else 0.0
        )
        extra_cpu += max(0.0, extra_cpu_seconds - baseline_cpu)
        extra_peak_rss = max(extra_peak_rss, extra_rss)
    wall_seconds = time.perf_counter() - started
    stderr = ""
    if stderr_file is not None:
        stderr_file.seek(0)
        stderr = stderr_file.read()[-3000:]
        stderr_file.close()
    returncode = process.returncode
    if returncode != 0:
        raise RuntimeError(
            f"command exited with {returncode}: "
            + " ".join(command)
            + "\n"
            + stderr.strip()[-3000:]
        )
    return Sample(
        wall_seconds=wall_seconds,
        cpu_seconds=cpu_seconds,
        peak_rss_kb=peak_rss,
        extra_cpu_seconds=extra_cpu,
        extra_peak_rss_kb=extra_peak_rss,
    )
