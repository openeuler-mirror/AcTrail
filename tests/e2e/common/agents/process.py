"""Own agent processes and task queues without interpreting agent protocols."""
from __future__ import annotations

import os
import signal
import subprocess
from concurrent.futures import Future, ThreadPoolExecutor, wait
from contextlib import ExitStack
from threading import Lock, Thread
from typing import Callable

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchResult
from tests.e2e.common.workspace import Workspace
from .base import AgentResult


class AgentProcess:
    """适配器通过组合复用进程清理和排队，协议处理仍由适配器提供。"""

    def __init__(self, shutdown_timeout: float):
        self._shutdown_timeout = shutdown_timeout
        self.launch_result: LaunchResult | None = None
        self._process: subprocess.Popen[str] | None = None
        self.stderr: list[str] = []
        self._executor: ThreadPoolExecutor | None = None
        self._tasks: list[Future[AgentResult]] = []
        self._lock = Lock()
        self._closing = False
        self._stderr_reader: Thread | None = None
        self._resources = ExitStack()

    @property
    def process(self) -> subprocess.Popen[str]:
        if self._process is None:
            raise RuntimeError("Agent has not been launched")
        return self._process

    def launch(self, command, *, environment, cwd) -> None:
        self._launch(command, environment=environment, cwd=cwd, controlled=True)

    def launch_direct(self, command, *, environment, cwd) -> None:
        """可用性探测直接运行 Agent，复用同一套管道和进程清理。"""
        self._launch(command, environment=environment, cwd=cwd, controlled=False)

    def _launch(self, command, *, environment, cwd, controlled: bool) -> None:
        if self._process is not None:
            raise RuntimeError("Agent has already been launched")
        try:
            workspace = self._resources.enter_context(Workspace.scope(cwd))
            if controlled:
                self.launch_result = ActrailRuntime.current().launch_with_ctl(
                    command, environment=environment, cwd=workspace.path,
                )
                process = self.launch_result.process
            else:
                process = subprocess.Popen(
                    [str(argument) for argument in command], env=environment, cwd=workspace.path,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    text=True, encoding="utf-8", start_new_session=True,
                )
            self._start_readers(process)
        except BaseException as error:
            self.__exit__(type(error), error, error.__traceback__)
            raise

    @property
    def returncode(self) -> int | None:
        return None if self._process is None else self._process.poll()

    @property
    def diagnostics(self) -> str:
        return "".join(self.stderr)

    @property
    def command(self) -> tuple[str, ...]:
        return () if self._process is None else tuple(str(arg) for arg in self._process.args)

    def _start_readers(self, process: subprocess.Popen[str]) -> None:
        self._process = process
        self._stderr_reader = Thread(target=self._read_stderr, daemon=True)
        self._stderr_reader.start()
        self._executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="agent")

    def submit(self, operation: Callable[[], AgentResult], *, single_task: bool = False) -> None:
        with self._lock:
            if self._executor is None or self._closing:
                raise RuntimeError("Agent is not running")
            if single_task and self._tasks:
                raise RuntimeError("This launch accepts one task")
            self._tasks.append(self._executor.submit(self._run, operation))

    def join_all(self, timeout: float | None) -> None:
        with self._lock:
            tasks = tuple(self._tasks)
        _, pending = wait(tasks, timeout=timeout)
        if pending:
            raise TimeoutError("Timed out waiting for agent tasks")

    @property
    def results(self) -> list[AgentResult]:
        with self._lock:
            tasks = tuple(self._tasks)
        return [task.result() for task in tasks if task.done() and not task.cancelled()]

    def close(self) -> None:
        self.__exit__(None, None, None)

    def __exit__(self, exception_type, exception, traceback) -> None:
        """先停止所有使用目录的进程，再释放本次拥有的 Workspace。"""
        try:
            self._close_process()
        except BaseException as error:
            if exception is None:
                self._resources.__exit__(type(error), error, error.__traceback__)
                raise
            exception.add_note(f"Agent process cleanup failed: {error}")
        self._resources.__exit__(exception_type, exception, traceback)

    def _close_process(self) -> None:
        with self._lock:
            self._closing = True
            active = any(task.running() for task in self._tasks)
        if self._executor is not None:
            self._executor.shutdown(wait=False, cancel_futures=True)
        if self._process is not None:
            process = self.process
            # 活跃任务可能持有 stdin 的写锁，先终止进程解除阻塞。
            if active:
                self._signal(signal.SIGTERM)
            elif process.stdin is not None and not process.stdin.closed:
                try:
                    process.stdin.close()
                except BrokenPipeError:
                    pass
            self._wait_for_exit()
            # 主进程退出后，结束仍持有管道的后代，再等待读取线程。
            self._signal(signal.SIGKILL)
        if self._executor is not None:
            self._executor.shutdown(wait=True, cancel_futures=True)
        if self._stderr_reader is not None:
            self._stderr_reader.join(timeout=self._shutdown_timeout)
            if self._stderr_reader.is_alive():
                self._signal(signal.SIGKILL)
                self._stderr_reader.join()
        if self._process is not None:
            for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
                if stream is not None:
                    stream.close()

    def _run(self, operation: Callable[[], AgentResult]) -> AgentResult:
        with self._lock:
            if self._closing:
                return AgentResult("", False, "Agent is closing")
        try:
            return operation()
        except Exception as error:
            with self._lock:
                self._closing = True
            self._signal(signal.SIGTERM)
            self._wait_for_exit()
            return AgentResult("", False, str(error))

    def _read_stderr(self) -> None:
        for line in self.process.stderr:
            self.stderr.append(line)

    def _wait_for_exit(self) -> None:
        try:
            self.process.wait(timeout=self._shutdown_timeout)
        except subprocess.TimeoutExpired:
            self._signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=self._shutdown_timeout)
            except subprocess.TimeoutExpired:
                self._signal(signal.SIGKILL)
                self.process.wait()

    def _signal(self, number: int) -> None:
        try:
            os.killpg(self.process.pid, number)
        except ProcessLookupError:
            pass
