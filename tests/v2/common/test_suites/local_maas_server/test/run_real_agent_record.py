#!/usr/bin/env python3
"""Record a real agent run into a replayable local MaaS scenario.

Usage:
  python3 test/run_real_agent_record.py \\
      --agent xiaoo \\
      --prompt "只读分析 ..." [--prompt-file prompt.txt] \\
      --max-turns 60 [--tools file_read,glob,grep] [--name recorded-xiaoo]
  python3 test/run_real_agent_record.py \\
      --agent opencode \\
      --prompt "只读分析 ..." [--prompt-file prompt.txt] \\
      [--tools read,glob,grep] [--name recorded-opencode]

The finalize artifact lands in
<templates-dir>/recorded/<name>-<timestamp>-<hash5>.json and its scenario id
is printed at the end. DEEPSEEK_API_KEY is used as the upstream credential.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from abc import ABC, abstractmethod
from pathlib import Path
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from test.e2e_support import MaaSServerProcess, REPO, _free_port  # noqa: E402


DEFAULT_MODEL = "deepseek-v4-flash"
DEFAULT_TOOLS_XIAOO = "file_read,glob,grep"
DEFAULT_TOOLS_OPENCODE = "read,glob,grep"


class AgentRecordBackend(ABC):
    """How one real agent talks to the recording server."""

    def __init__(
        self,
        binary: str,
        model: str,
        tools: tuple[str, ...],
    ):
        self.binary = binary
        self.model = model
        self.tools = tools
        self._port: int | None = None
        self._api_key: str | None = None

    @property
    @abstractmethod
    def name(self) -> str:
        raise NotImplementedError

    def prepare(
        self,
        workdir: Path,
        server: MaaSServerProcess,
        session: dict[str, Any],
    ) -> None:
        del workdir
        self._port = server.port
        self._api_key = session["api_key"]

    @abstractmethod
    def command(self, prompt: str, max_turns: int) -> list[str]:
        raise NotImplementedError

    def run_cwd(self, workdir: Path) -> Path:
        return REPO

    def run_env(self, workdir: Path) -> dict[str, str]:
        return {}


class XiaooBackend(AgentRecordBackend):
    @property
    def name(self) -> str:
        return "xiaoo"

    def command(self, prompt: str, max_turns: int) -> list[str]:
        assert self._port is not None
        assert self._api_key is not None
        return [
            self.binary,
            "--cli",
            "run",
            "-p",
            prompt,
            "--provider",
            "openai",
            "--api-base",
            f"http://127.0.0.1:{self._port}",
            "--api-key",
            self._api_key,
            "--model",
            self.model,
            "--max-turns",
            str(max_turns),
            "--tools",
            ",".join(self.tools),
            "--debug",
        ]


class OpencodeBackend(AgentRecordBackend):
    @property
    def name(self) -> str:
        return "opencode"

    def prepare(
        self,
        workdir: Path,
        server: MaaSServerProcess,
        session: dict[str, Any],
    ) -> None:
        super().prepare(workdir, server, session)
        config = {
            "provider": {
                "bench": {
                    "npm": "@ai-sdk/openai-compatible",
                    "options": {
                        "baseURL": (
                            f"http://127.0.0.1:{server.port}/v1"
                        ),
                        "apiKey": session["api_key"],
                    },
                    "models": {
                        self.model: {"name": self.model},
                    },
                }
            }
        }
        (workdir / "opencode.json").write_text(
            json.dumps(config, indent=2) + "\n",
            encoding="utf-8",
        )

    def command(self, prompt: str, max_turns: int) -> list[str]:
        del max_turns  # opencode runs to completion; no turn limit flag
        return [
            self.binary,
            "run",
            "--model",
            f"bench/{self.model}",
            "--auto",
            prompt,
        ]

    def run_cwd(self, workdir: Path) -> Path:
        return workdir

    def run_env(self, workdir: Path) -> dict[str, str]:
        return {"PWD": str(workdir)}


class RealAgentRecord:
    def __init__(
        self,
        workdir: Path,
        *,
        backend: AgentRecordBackend,
        prompt: str,
        max_turns: int,
        tools: tuple[str, ...],
        name: str,
    ):
        self.workdir = workdir
        self.backend = backend
        self.prompt = prompt
        self.max_turns = max_turns
        self.tools = tools
        self.name = name
        self.recordings_dir = workdir / "recordings"
        self.agent_log = workdir / f"{backend.name}.log"
        self.record_server: MaaSServerProcess | None = None
        self.session: dict[str, Any] | None = None

    def run(self) -> None:
        api_key = os.environ.get("DEEPSEEK_API_KEY")
        if not api_key:
            raise RuntimeError("DEEPSEEK_API_KEY is required")
        agent_bin = shutil.which(self.backend.name)
        if agent_bin is None:
            raise RuntimeError(
                f"agent binary not found: {self.backend.name}"
            )

        self.record_server = MaaSServerProcess(
            [
                "record",
                "--disable-https",
                "--http-bind-port",
                str(_free_port()),
                "--recordings-dir",
                str(self.recordings_dir),
            ],
            workdir=REPO,
        )
        self.record_server.wait_ready()
        self._create_session(api_key)
        rounds = self._run_agent()
        finalized = self._finalize()
        self._report(rounds, finalized)
        if rounds < 1:
            raise RuntimeError("no rounds were recorded")

    def _create_session(self, upstream_api_key: str) -> None:
        assert self.record_server is not None
        status, body = self.record_server.request(
            "POST",
            "/record/sessions",
            document={
                "tools": list(self.tools),
                "upstream": {
                    "base_url": "https://api.deepseek.com",
                    "api_key": upstream_api_key,
                    "model": DEFAULT_MODEL,
                },
            },
        )
        assert status == 201, (status, body)
        self.session = json.loads(body)

    def _run_agent(self) -> int:
        assert self.record_server is not None
        assert self.session is not None
        self.backend.prepare(
            self.workdir,
            self.record_server,
            self.session,
        )
        run_cwd = self.backend.run_cwd(self.workdir)
        command = self.backend.command(
            self.prompt,
            self.max_turns,
        )
        with self.agent_log.open("wb") as log_file:
            process = subprocess.Popen(
                command,
                cwd=str(run_cwd),
                env={
                    **os.environ,
                    **self.backend.run_env(self.workdir),
                },
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )
            deadline = time.monotonic() + 3600
            while time.monotonic() < deadline:
                cache = Path(self.session["cache_file"])
                rounds = (
                    len(cache.read_text(encoding="utf-8").splitlines())
                    if cache.is_file()
                    else 0
                )
                print(
                    f"agent running... recorded rounds: {rounds}",
                    flush=True,
                )
                if process.poll() is not None:
                    break
                time.sleep(15)
            if process.poll() is None:
                print(
                    "agent still running after the deadline; "
                    "terminating to finalize",
                    flush=True,
                )
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        cache = Path(self.session["cache_file"])
        if not cache.is_file():
            return 0
        return len(cache.read_text(encoding="utf-8").splitlines())

    def _finalize(self) -> dict[str, Any]:
        assert self.record_server is not None
        assert self.session is not None
        status, body = self.record_server.request(
            "POST",
            f"/record/sessions/{self.session['session_id']}/finalize",
            document={"scenario_id": self.name},
        )
        assert status == 200, (status, body)
        return json.loads(body)

    def _report(
        self,
        rounds: int,
        finalized: dict[str, Any],
    ) -> None:
        kinds: dict[str, int] = {}
        tool_names: dict[str, int] = {}
        meta_file = Path(finalized["scenario_file"])
        artifact = meta_file.name.removesuffix(".meta.json")
        rounds_nodes: list[dict[str, Any]] = []
        for suffix in ("tool", "message"):
            round_file = meta_file.parent / f"{artifact}.{suffix}.jsonl"
            for line in round_file.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    rounds_nodes.append(json.loads(line))
        for node in rounds_nodes:
            response = node.get("response", {})
            for block in response.get("blocks", []):
                kinds[block["type"]] = kinds.get(block["type"], 0) + 1
                if block["type"] == "tool_call":
                    name = block["name"]
                    tool_names[name] = tool_names.get(name, 0) + 1
        print("=" * 60)
        print(f"scenario id:          {finalized['scenario_id']}")
        print(f"recorded rounds:      {rounds}")
        print(f"block kinds:          {kinds}")
        print(f"tool calls (canon):   {tool_names}")
        print(f"scenario meta:        {meta_file}")
        print(f"recordings dir:       {self.recordings_dir}")
        print(f"agent log:            {self.agent_log}")
        print("=" * 60)

    def cleanup(self) -> None:
        if self.record_server is not None:
            self.record_server.stop()
            self.record_server = None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Record a real agent run into a replayable scenario",
    )
    parser.add_argument(
        "--agent",
        choices=("xiaoo", "opencode"),
        default="xiaoo",
        help="agent to record (xiaoo or opencode)",
    )
    prompt_group = parser.add_mutually_exclusive_group(required=True)
    prompt_group.add_argument(
        "--prompt",
        help="inline prompt for the agent",
    )
    prompt_group.add_argument(
        "--prompt-file",
        type=Path,
        help="file containing the prompt for the agent",
    )
    parser.add_argument(
        "--max-turns",
        type=int,
        default=35,
        help="max turns allowed for the agent loop",
    )
    parser.add_argument(
        "--tools",
        default=None,
        help=(
            "comma-separated tool allowlist; defaults to "
            "file_read,glob,grep for xiaoo and read,glob,grep for opencode"
        ),
    )
    parser.add_argument(
        "--name",
        default=None,
        help=(
            "scenario id base; defaults to recorded-<agent>; "
            "final id appends timestamp and hash"
        ),
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    prompt = args.prompt
    if args.prompt_file is not None:
        try:
            prompt = args.prompt_file.read_text(encoding="utf-8")
        except OSError as error:
            raise SystemExit(
                f"cannot read --prompt-file {args.prompt_file}: {error}"
            )
    tools_text = args.tools or {
        "xiaoo": DEFAULT_TOOLS_XIAOO,
        "opencode": DEFAULT_TOOLS_OPENCODE,
    }[args.agent]
    tools = tuple(
        tool.strip()
        for tool in tools_text.split(",")
        if tool.strip()
    )
    if not tools:
        raise SystemExit("--tools must contain at least one tool")
    workdir = Path(tempfile.mkdtemp(prefix="agent-record-"))
    binary = shutil.which(args.agent)
    if binary is None:
        raise SystemExit(f"agent binary not found: {args.agent}")
    if args.agent == "xiaoo":
        backend: AgentRecordBackend = XiaooBackend(
            binary, DEFAULT_MODEL, tools
        )
    else:
        backend = OpencodeBackend(binary, DEFAULT_MODEL, tools)
    runner = RealAgentRecord(
        workdir,
        backend=backend,
        prompt=prompt,
        max_turns=args.max_turns,
        tools=tools,
        name=args.name or f"recorded-{args.agent}",
    )
    try:
        runner.run()
    finally:
        runner.cleanup()
    print("RECORD_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
