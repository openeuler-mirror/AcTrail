"""Encapsulate Pi RPC turns and context compaction."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult, ToolAccess
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from tests.e2e.common.process import ManagedProcess
from tests.e2e.common.workspace import Workspace
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess


class PiAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """读取 Pi 的 RPC 状态，取得实际选择的模型，不发送推理请求。"""
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("PI_E2E_BINARY", "pi")
        if selected is None:
            return None
        command = (str(selected), "--mode", "rpc", "--no-session", "--offline")
        workspace = Workspace.current()
        try:
            process = subprocess.Popen(
                command, env=discovery.environment(selected),
                cwd=workspace.path if workspace is not None else None,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, start_new_session=True,
            )
        except OSError:
            return None
        managed = ManagedProcess(command, process)
        try:
            identifier = "default-model-query"
            process.stdin.write(json.dumps({"id": identifier, "type": "get_state"}) + "\n")
            process.stdin.flush()
            managed.wait_for_output(f'"id":"{identifier}"', timeout=MODEL_DEFAULTS.discovery_timeout_seconds)
            # Pi 的 EOF 会立即触发退出，必须先收到状态响应再关闭输入。
            process.stdin.close()
            result = managed.wait(timeout=MODEL_DEFAULTS.discovery_timeout_seconds)
            for line in result.stdout.splitlines():
                try:
                    response = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if response.get("id") == identifier and response.get("success"):
                    model = response.get("data", {}).get("model")
                    if model is not None:
                        return f'{model["provider"]}/{model["id"]}'
            return None
        except (OSError, TimeoutError, RuntimeError):
            return None
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            managed.terminate(grace_seconds=DEFAULTS.shutdown_timeout_seconds)

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("PI_E2E_BINARY", "pi")
        if self._binary is None:
            raise FileNotFoundError("Pi executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)
        self._stream_mode = True
        self._next_id = 0

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _configured_arguments(self, options: LaunchOptions | None) -> list[str]:
        arguments = AgentArguments(self._arguments)
        if options is None:
            return arguments.values
        if options.tool_access is not None:
            arguments.remove(values=("--tools", "-t", "--exclude-tools", "-xt"),
                             switches=("--no-tools", "-nt", "--no-builtin-tools", "-nbt"))
            if options.tool_access is ToolAccess.NONE:
                arguments.append("--no-tools")
            elif options.tool_access is ToolAccess.READ_ONLY:
                arguments.remove(switches=("--no-mcp",))
                arguments.append("--tools", "read,grep,find,ls", "--no-mcp")
            else:
                arguments.append("--tools", "read,bash,edit,write,grep,find,ls")
        return arguments.values

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        arguments = self._configured_arguments(options)
        self._stream_mode = stream_mode
        mode = ["--mode", "rpc"] if stream_mode else ["-p", "--mode", "json"]
        launch_process(
            [self._binary, *arguments, *mode, "--no-session"],
            environment=self._environment, cwd=self._cwd,
        )
        return self

    def run(self, prompt: str) -> None:
        self._session.submit(lambda: self._execute_prompt(prompt), single_task=not self._stream_mode)

    def compact(self) -> None:
        if not self._stream_mode:
            raise NotImplementedError("Pi compaction requires RPC mode")
        self._session.submit(self._execute_compact)

    def _send(self, command: str, **fields) -> str:
        """发送 RPC 命令并返回请求 ID。"""
        self._next_id += 1
        identifier = str(self._next_id)
        process = self._session.process
        process.stdin.write(json.dumps({"id": identifier, "type": command, **fields}) + "\n")
        process.stdin.flush()
        return identifier

    def _execute_prompt(self, prompt: str) -> AgentResult:
        process = self._session.process
        identifier = None
        acknowledged = not self._stream_mode
        completed = False
        reply_text = ""
        if self._stream_mode:
            identifier = self._send("prompt", message=prompt)
        else:
            process.stdin.write(prompt)
            process.stdin.close()
        failure = None
        # prompt 响应确认接收，agent_end 确认任务完成，两者均须消费。
        while not (completed and acknowledged):
            line = process.stdout.readline()
            if not line:
                raise RuntimeError("Pi exited before the task completed")
            event = json.loads(line)
            if event.get("type") == "response":
                if event.get("id") == identifier:
                    acknowledged = True
                    if not event.get("success"):
                        return AgentResult("", False, event.get("error", "Pi prompt failed"))
                continue
            if event.get("type") != "agent_end":
                continue
            completed = True
            for message in event.get("messages", []):
                if message.get("role") != "assistant":
                    continue
                # agent_end includes earlier assistant messages in the same turn.
                reply_text = "".join(
                    block.get("text", "")
                    for block in message.get("content", [])
                    if block.get("type") == "text"
                )
                if message.get("stopReason") in {"error", "aborted"}:
                    failure = message.get("errorMessage") or message["stopReason"]
            if not self._stream_mode:
                process.stdout.read()
                code = process.wait()
                if code:
                    failure = failure or f"Pi exited with {code}"
        return AgentResult(reply_text, failure is None, failure)

    def _execute_compact(self) -> AgentResult:
        identifier = self._send("compact")
        for line in self._session.process.stdout:
            event = json.loads(line)
            if event.get("type") != "response" or event.get("id") != identifier:
                continue
            succeeded = event.get("success") is True
            data = event.get("data") or {}
            return AgentResult(data.get("summary", ""), succeeded,
                               None if succeeded else event.get("error", "Pi compaction failed"))
        raise RuntimeError("Pi exited before compaction completed")

    def join_all(self, timeout: float | None, **kwargs) -> None:
        self._session.join_all(timeout)

    @property
    def results(self) -> list[AgentResult]:
        return self._session.results

    @property
    def launch_result(self) -> LaunchResult | None:
        return self._session.launch_result

    @property
    def returncode(self) -> int | None:
        return self._session.returncode

    @property
    def diagnostics(self) -> str:
        return self._session.diagnostics

    @property
    def command(self) -> tuple[str, ...]:
        return self._session.command

    def __exit__(self, exception_type, exception, traceback) -> None:
        self._session.__exit__(exception_type, exception, traceback)
