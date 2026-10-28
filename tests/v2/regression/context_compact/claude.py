from __future__ import annotations

import errno
import fcntl
import os
import pty
import re
import select
import signal
import shutil
import struct
import subprocess
import termios
import time
import tty
import uuid
from pathlib import Path
from typing import BinaryIO

from tests.v2.common.actrail_runtime import ActrailRuntime

from .config import ContextCompactConfig
from .scenario import AgentRun, AgentSpec, ConversationScenario


class ClaudeSessionDriver:
    """Run the three prompts in one real Claude Code interactive session."""

    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    _TITLE_PATTERN = re.compile(r"\x1b\](?:0|2);([^\x07\x1b]*)(?:\x07|\x1b\\)")
    _CSI_PATTERN = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
    _TRUST_PATTERN = re.compile(r"Yes,\s*I\s*trust\s*this\s*folder")

    def __init__(
        self, config: ContextCompactConfig, runtime: ActrailRuntime,
        spec: AgentSpec, scenario: ConversationScenario,
    ):
        self._config = config
        self._runtime = runtime
        self._spec = spec
        self._scenario = scenario
        self._directory = config.work_dir / scenario.name
        self._process: subprocess.Popen[bytes] | None = None
        self._master: int | None = None
        self._log: BinaryIO | None = None
        self._output = bytearray()
        self._session_id = str(uuid.uuid4())

    def run(self) -> AgentRun:
        self._directory.mkdir(parents=True, exist_ok=True)
        (self._directory / "workspace").mkdir(exist_ok=True)
        with (self._directory / "claude.log").open("wb") as log:
            self._log = log
            try:
                self._start()
                trace_id = self._wait_for_startup()
                for index, prompt in enumerate(self._scenario.prompts):
                    offset = self._submit(prompt)
                    self._wait_for_turn(index, offset)
                return AgentRun(trace_id)
            finally:
                try:
                    self._close()
                    self._archive_session()
                finally:
                    self._log = None

    def _start(self) -> None:
        master, slave = pty.openpty()
        self._master = master
        try:
            tty.setraw(slave)
            size = struct.pack(
                "HHHH", self._config.terminal_rows, self._config.terminal_columns, 0, 0,
            )
            fcntl.ioctl(slave, termios.TIOCSWINSZ, size)
            environment = dict(self._spec.environment)
            environment.setdefault("TERM", "xterm-256color")
            self._process = subprocess.Popen(
                [*self._runtime.control_command("launch", "--", self._spec.binary),
                 "--session-id", self._session_id,
                 *self._spec.arguments],
                cwd=self._directory / "workspace",
                env=environment,
                stdin=slave, stdout=slave, stderr=slave,
                start_new_session=True,
            )
            os.set_blocking(master, False)
        finally:
            os.close(slave)

    def _wait_for_startup(self) -> int:
        deadline = time.monotonic() + self._config.startup_timeout_seconds
        trusted = False
        title_offset = 0
        while time.monotonic() < deadline:
            self._pump(deadline)
            output = self._output.decode("utf-8", errors="replace")
            trace_ids = set(self._TRACE_PATTERN.findall(output))
            if len(trace_ids) > 1:
                raise RuntimeError(f"Claude launch reported multiple trace IDs: {trace_ids}")
            if not trusted and self._TRUST_PATTERN.search(self._CSI_PATTERN.sub("", output)):
                self._trust_workspace(deadline)
                trusted = True
                title_offset = len(self._output)
                continue
            titles = self._TITLE_PATTERN.findall(
                self._output[title_offset:].decode("utf-8", errors="replace")
            )
            if titles and titles[-1].startswith("✳") and len(trace_ids) == 1:
                return int(next(iter(trace_ids)))
        raise TimeoutError(f"Claude startup timed out; PTY tail={self._tail()!r}")

    def _trust_workspace(self, deadline: float) -> None:
        # Select the empty workspace created by this driver, then wait for
        # the menu to render the selection before confirming it.
        offset = len(self._output)
        self._write(b"\x1b[B")
        while time.monotonic() < deadline:
            self._pump(deadline)
            if "❯" in self._output[offset:].decode("utf-8", errors="replace"):
                self._write(b"\r")
                return
        raise TimeoutError("Claude did not render the workspace trust selection")

    def _submit(self, prompt: str) -> int:
        self._write(prompt.encode("utf-8"))
        # Let the TUI render the pasted text before pressing Enter.
        deadline = time.monotonic() + self._config.drain_interval_seconds
        while time.monotonic() < deadline:
            self._pump(deadline)
        offset = len(self._output)
        self._write(b"\r")
        return offset

    def _wait_for_turn(self, index: int, output_offset: int) -> None:
        deadline = time.monotonic() + self._config.turn_timeout_seconds
        while time.monotonic() < deadline:
            self._pump(deadline)
            titles = self._TITLE_PATTERN.findall(
                self._output[output_offset:].decode("utf-8", errors="replace")
            )
            if titles and titles[-1].startswith("✳") and any(not title.startswith("✳") for title in titles[:-1]):
                return
        raise TimeoutError(f"Claude did not complete turn {index + 1}; PTY tail={self._tail()!r}")

    def _pump(self, deadline: float) -> None:
        master = self._required_master()
        readable, _, _ = select.select(
            [master], [], [],
            min(self._config.drain_interval_seconds, max(deadline - time.monotonic(), 0)),
        )
        if readable:
            self._read_available()
        if self._process is not None and self._process.poll() is not None:
            self._read_available()
            raise RuntimeError(
                f"Claude exited with {self._process.returncode}; PTY tail={self._tail()!r}"
            )

    def _read_available(self) -> None:
        while True:
            try:
                chunk = os.read(self._required_master(), 64 * 1024)
            except BlockingIOError:
                return
            except OSError as error:
                if error.errno == errno.EIO:
                    return
                raise
            if not chunk:
                return
            self._output.extend(chunk)
            if self._log is not None:
                self._log.write(chunk)
                self._log.flush()

    def _write(self, value: bytes) -> None:
        written = 0
        deadline = time.monotonic() + self._config.turn_timeout_seconds
        while written < len(value):
            try:
                written += os.write(self._required_master(), value[written:])
            except BlockingIOError:
                self._pump(deadline)
                if time.monotonic() >= deadline:
                    raise TimeoutError("Claude did not consume terminal input")

    def _required_master(self) -> int:
        if self._master is None:
            raise RuntimeError("Claude terminal has not been opened")
        return self._master

    def _archive_session(self) -> None:
        configured = self._spec.environment.get("CLAUDE_CONFIG_DIR")
        config_root = Path(configured) if configured else Path(self._spec.environment["HOME"]) / ".claude"
        for index, path in enumerate((config_root / "projects").glob(f"*/{self._session_id}.jsonl")):
            shutil.move(str(path), self._directory / f"claude-session-{index}.jsonl")

    def _tail(self) -> str:
        return self._output[-4000:].decode("utf-8", errors="replace")

    def _close(self) -> None:
        try:
            process = self._process
            if process is not None and process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=self._config.shutdown_timeout_seconds)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                except ProcessLookupError:
                    process.wait()
            if self._master is not None:
                self._read_available()
        finally:
            if self._master is not None:
                os.close(self._master)
                self._master = None
