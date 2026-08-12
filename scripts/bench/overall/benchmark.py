"""Orchestration for the overall replay benchmark."""

from __future__ import annotations

import argparse
import json
import shutil
import tempfile
import time
from pathlib import Path
from typing import Sequence

from test.e2e_support import MaaSServerProcess, _free_port

from scenario import ScenarioConfigurationError
from scenario.model import ScenarioMeta
from scenario.scenario_generator import ScenarioRegistry

from .agents import AgentBackend
from .agents import opencode as opencode_agent
from .agents import xiaoo as xiaoo_agent
from .console import print_scenario_list
from .measurement import ProcTreeSampler, Sample, measure_command
from .reporting import Report, print_comparison
from .runtime import (
    ReleaseBuild,
    prepare_actrail,
    stop_actrail,
    storage_footprint_bytes,
)


REPO_ROOT = Path(__file__).resolve().parents[3]


def scenario_rounds(scenario_id: str) -> int:
    try:
        meta = ScenarioRegistry.from_environment().scenario_meta(
            scenario_id
        )
    except ScenarioConfigurationError as error:
        raise SystemExit(str(error))
    if meta.rounds is None:
        raise SystemExit(
            f"scenario {scenario_id!r} has no countable rounds "
            "(non-recorded scenario); pass --max-turns"
        )
    return meta.rounds


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Benchmark recorded-scenario replay: bare xiaoo vs actrail",
    )
    parser.add_argument(
        "--scenario",
        default=None,
        help="scenario id to benchmark (required; see --list-scenarios)",
    )
    parser.add_argument(
        "--list-scenarios",
        action="store_true",
        help="list available scenario ids and exit without running",
    )
    parser.add_argument(
        "--prompt",
        default=xiaoo_agent.DEFAULT_PROMPT,
        help="prompt sent to agent (replay ignores its content)",
    )
    parser.add_argument(
        "--max-turns",
        type=int,
        default=0,
        help=(
            "max turns for xiaoo; defaults to the entire scenario "
            "(tool + message rounds), i.e. one round = one full replay"
        ),
    )
    parser.add_argument(
        "--rounds",
        type=int,
        default=5,
        help=(
            "measured rounds per case (alternating order); one separate "
            "warmup run per case is discarded"
        ),
    )
    parser.add_argument(
        "--no-tls-capture",
        action="store_true",
        help="disable payload.tls capture in the actraild config patch",
    )
    parser.add_argument(
        "--no-stdio-capture",
        action="store_true",
        help="disable payload.stdio capture in the actraild config patch",
    )
    parser.add_argument(
        "--no-seccomp",
        action="store_true",
        help=(
            "empty the TLS/socket seccomp syscall lists so observed processes "
            "never hit seccomp user-notify (payloads on those paths are lost)"
        ),
    )
    parser.add_argument(
        "--tpot-ms",
        type=float,
        default=3.0,
        help=(
            "replay server delay per SSE frame (milliseconds); a small "
            "nonzero value models realistic LLM streaming pacing"
        ),
    )
    parser.add_argument(
        "--build-timeout-seconds",
        type=float,
        default=3600.0,
        help=(
            "timeout for the forced `cargo build --release` at startup "
            "(default: 3600)"
        ),
    )
    parser.add_argument(
        "--lazy-load-size",
        type=int,
        default=0,
        help=(
            "recorded rounds read batch for the replay server: "
            "0 eager, N>0 streams N lines at a time (default: 0)"
        ),
    )
    parser.add_argument(
        "--settle-ms",
        type=float,
        default=1000.0,
        help=(
            "pause after POST /reset before starting the next case, letting "
            "the previous trace finalize and connections drain"
        ),
    )
    parser.add_argument(
        "--agent",
        choices=("xiaoo", "opencode"),
        default="opencode",
        help="agent backend to benchmark (default: opencode)",
    )
    parser.add_argument(
        "--agent-binary",
        default="",
        help=(
            "path to the --agent binary override "
            "(default: resolved from PATH)"
        ),
    )
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=Path("/usr/local/bin"),
        help="directory containing actraild/actrailctl",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=(
            Path(__file__).resolve().parent
            / "out"
            / f"bench-overall-{time.strftime('%Y%m%d%H%M%S')}.json"
        ),
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = create_parser().parse_args(argv)
    if args.list_scenarios:
        print_scenario_list(_available_scenarios())
        return 0
    if not args.scenario:
        raise SystemExit(
            "--scenario is required; available scenarios:\n"
            + _scenario_listing_text()
        )
    max_turns = args.max_turns or scenario_rounds(args.scenario)
    agent: AgentBackend
    if args.agent == "xiaoo":
        agent = xiaoo_agent.backend(args.agent_binary)
    else:
        agent = opencode_agent.backend(args.agent_binary)
    if agent.binary is None:
        raise SystemExit(
            f"{agent.name} not found; pass --agent-binary explicitly"
        )
    if args.rounds < 1:
        raise SystemExit("--rounds must be at least 1")

    report = Report(
        scenario=args.scenario,
        agent=args.agent,
        rounds=args.rounds,
        max_turns=max_turns,
    )
    actrail_work_dir = Path(tempfile.mkdtemp(prefix="bench-actrail-"))
    commit = ReleaseBuild(REPO_ROOT).ensure(
        timeout_seconds=args.build_timeout_seconds,
    )
    report.commit_id = commit["id"]
    report.commit_title = commit["title"]
    print(
        f"commit: {commit['id'][:8]} {commit['title']}",
        flush=True,
    )
    actraild_pid = prepare_actrail(
        actrail_work_dir,
        args.bin_dir,
        no_tls_capture=args.no_tls_capture,
        no_stdio_capture=args.no_stdio_capture,
        no_seccomp=args.no_seccomp,
    )
    replay_server = MaaSServerProcess(
        [
            "replay",
            "--disable-https",
            "--http-bind-port",
            str(_free_port()),
            "--scenario",
            args.scenario,
            "--tpot-milliseconds",
            str(args.tpot_ms),
            "--lazy-load-size",
            str(args.lazy_load_size),
        ],
        workdir=REPO_ROOT,
    )
    replay_server.wait_ready()
    agent.prepare_workdir(actrail_work_dir, replay_server.port)
    agent_cwd = agent.working_directory(actrail_work_dir) or REPO_ROOT
    last_bare_wall_ms: float | None = None

    def run_case(is_bare: bool) -> Sample:
        replay_server.request("POST", "/reset")
        time.sleep(args.settle_ms / 1000.0)
        extra_baselines: dict[int, float] = {}
        if actraild_pid:
            baseline_cpu, _, _ = ProcTreeSampler(actraild_pid).sample()
            extra_baselines[actraild_pid] = baseline_cpu
            if not is_bare:
                report.actrail_baselines_ms.append(baseline_cpu * 1000)
        if is_bare:
            command = agent.command(replay_server.port, args.prompt, max_turns)
        else:
            command = [
                str(args.bin_dir / "actrailctl"),
                "--config",
                str(actrail_work_dir / "actraild.conf"),
                "launch",
                "--",
                *agent.command(replay_server.port, args.prompt, max_turns),
            ]
        return measure_command(
            command,
            cwd=agent_cwd,
            extra_pids=(actraild_pid,) if actraild_pid else (),
            extra_baselines=extra_baselines,
            timeout_seconds=agent.case_timeout_seconds,
        )

    try:
        print(
            f"warmup: bare + actrail (discarded) [max_turns={max_turns}]",
            flush=True,
        )
        run_case(True)
        run_case(False)
        for index in range(args.rounds):
            order = (True, False) if index % 2 == 0 else (False, True)
            round_walls: dict[bool, float] = {}
            for is_bare in order:
                sample = run_case(is_bare)
                if is_bare:
                    last_bare_wall_ms = sample.wall_ms
                if (
                    not is_bare
                    and last_bare_wall_ms is not None
                    and sample.wall_ms > 2.5 * last_bare_wall_ms
                ):
                    dump_dir = Path(
                        f"/tmp/bench-spike-dump-"
                        f"{time.strftime('%Y%m%d%H%M%S')}-round{index + 1}"
                    )
                    dump_dir.mkdir(parents=True, exist_ok=True)
                    for relative in (
                        "log/actraild.log",
                        "data/actrail.sqlite",
                        "data/actrail.sqlite-wal",
                    ):
                        source = actrail_work_dir / relative
                        if source.exists():
                            shutil.copy2(source, dump_dir / source.name)
                    print(
                        f"  [spike] daemon diagnostics copied to {dump_dir}",
                        flush=True,
                    )
                round_walls[is_bare] = sample.wall_ms
                if is_bare:
                    report.bare_samples.append(sample)
                else:
                    report.actrail_samples.append(sample)
            print(
                f"round {index + 1}/{args.rounds}  "
                f"bare {round_walls[True]:.0f}ms  "
                f"actrail {round_walls[False]:.0f}ms",
                flush=True,
            )
        time.sleep(args.settle_ms / 1000.0)
        report.storage_footprint_bytes = storage_footprint_bytes(
            actrail_work_dir
        )
    finally:
        replay_server.stop()
        stop_actrail(actrail_work_dir, args.bin_dir)

    if not report.bare_samples or not report.actrail_samples:
        raise SystemExit("benchmark produced no samples")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(report.to_dict(), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print_comparison(report.bare_samples, report.actrail_samples)
    print(f"report written to {args.out}")
    return 0


def _available_scenarios() -> tuple[ScenarioMeta, ...]:
    try:
        return ScenarioRegistry.from_environment().available_scenarios()
    except ScenarioConfigurationError as error:
        raise SystemExit(str(error))


def _scenario_listing_text() -> str:
    lines = ["available scenarios:"]
    for scenario in _available_scenarios():
        lines.append(f"  {scenario.scenario_id}")
        lines.append(f"    {scenario.description}")
    return "\n".join(lines)
