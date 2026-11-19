"""Encapsulate Codex exec and keep inference in the observed process tree."""
from __future__ import annotations

import json
import subprocess
from collections import deque
import os
from pathlib import Path
from typing import ClassVar, Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult, ToolAccess
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from .process import AgentProcess


class CodexAgent(Agent):
    _no_daemon_support: ClassVar[dict[Path, bool]] = {}

    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """通过所选 Codex 的模型目录查询当前 provider 可用的模型。"""
        configured = os.environ.get("CODEX_E2E_MODEL")
        if configured:
            return configured
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("CODEX_E2E_BINARY", "codex")
        if selected is None:
            return None
        try:
            completed = subprocess.run(
                [str(selected), "debug", "models"], capture_output=True, text=True,
                timeout=MODEL_DEFAULTS.discovery_timeout_seconds,
                env=discovery.codex_environment(selected),
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if completed.returncode != 0:
            return None
        try:
            document = json.loads(completed.stdout)
        except json.JSONDecodeError:
            return None
        models = document.get("models") if isinstance(document, dict) else None
        if not isinstance(models, list):
            return None
        for model in models:
            if not isinstance(model, dict):
                continue
            slug = model.get("slug")
            if isinstance(slug, str) and slug and model.get("supported_in_api") is True and model.get("visibility") != "hidden":
                return slug
        return None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("CODEX_E2E_BINARY", "codex")
        if self._binary is None:
            raise FileNotFoundError("Codex executable not found")
        self._environment = environment if environment is not None else discovery.codex_environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)
        self._stream_mode = True
        self._next_id = 0
        self._thread_id: str | None = None
        self._notifications: deque[dict] = deque()

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _supports_no_daemon(self) -> bool:
        # Availability checks and observed launches can use different symlinks to the same CLI.
        binary = self._binary.resolve()
        if binary not in self._no_daemon_support:
            help_result = subprocess.run(
                [str(self._binary), "--help"],
                env=self._environment,
                capture_output=True,
                text=True,
                timeout=MODEL_DEFAULTS.discovery_timeout_seconds,
                check=True,
            )
            self._no_daemon_support[binary] = any(
                line.split(maxsplit=1)[:1] == ["--no-daemon"]
                for line in help_result.stdout.splitlines()
            )
        return self._no_daemon_support[binary]

    def _configured_arguments(self, options: LaunchOptions | None) -> list[str]:
        arguments = AgentArguments(self._arguments)
        settings = {
            value.partition("=")[0].strip()
            for index, value in enumerate(arguments.values)
            if index and arguments.values[index - 1] in ("-c", "--config")
        }
        settings.update(
            value.removeprefix("--config=").partition("=")[0].strip()
            for value in arguments.values if value.startswith("--config=")
        )
        model = self._environment.get("CODEX_E2E_MODEL")
        explicit_model = any(
            value in ("-m", "--model") or value.startswith("--model=")
            for value in arguments.values
        )
        if model and not explicit_model and "model" not in settings:
            arguments.append("--model", model)
        effort = self._environment.get("CODEX_E2E_REASONING_EFFORT")
        if effort and "model_reasoning_effort" not in settings:
            arguments.append("-c", f"model_reasoning_effort={json.dumps(effort)}")
        if options is None or options.tool_access not in (ToolAccess.READ_ONLY, ToolAccess.WRITABLE):
            return arguments.values
        arguments.remove(values=("--sandbox", "-s"),
                         switches=("--full-auto", "--dangerously-bypass-approvals-and-sandbox", "--yolo"))
        # Keep unrelated -c settings while replacing an explicit sandbox_mode override.
        retained = []
        index = 0
        while index < len(arguments.values):
            argument = arguments.values[index]
            if argument in ("-c", "--config") and index + 1 < len(arguments.values):
                value = arguments.values[index + 1]
                if value.partition("=")[0].strip() != "sandbox_mode":
                    retained.extend((argument, value))
                index += 2
                continue
            if not argument.startswith(("--config=sandbox_mode=", "-csandbox_mode=")):
                retained.append(argument)
            index += 1
        sandbox = "read-only" if options.tool_access is ToolAccess.READ_ONLY else "workspace-write"
        return [*retained, "-c", f'sandbox_mode="{sandbox}"']

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        arguments = self._configured_arguments(options)
        self._stream_mode = stream_mode
        if not stream_mode and "--skip-git-repo-check" not in arguments:
            arguments.append("--skip-git-repo-check")
        daemon_options = ("--no-daemon",) if self._supports_no_daemon() else ()
        mode = [*arguments, "app-server"] if stream_mode else [
            "exec", *arguments, "--ephemeral", "--json", "-",
        ]
        launch_process(
            [self._binary, *daemon_options, *mode],
            environment=self._environment, cwd=self._cwd,
        )
        return self

    def run(self, prompt: str) -> None:
        if self._stream_mode:
            self._session.submit(lambda: self._execute_stream(prompt), single_task=False)
        else:
            self._session.submit(lambda: self._execute(prompt), single_task=True)

    def compact(self) -> None:
        if not self._stream_mode:
            raise NotImplementedError("Codex compaction requires app-server mode")
        self._session.submit(self._execute_compact)

    def _send(self, method: str, params: dict) -> int:
        """发送 RPC 请求并返回请求 ID。"""
        self._next_id += 1
        self._write({"id": self._next_id, "method": method, "params": params})
        return self._next_id

    def _write(self, message: dict) -> None:
        process = self._session.process
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()

    def _read(self) -> dict:
        line = self._session.process.stdout.readline()
        if not line:
            raise RuntimeError("Codex exited before the task completed")
        return json.loads(line)

    def _request(self, method: str, params: dict) -> dict:
        identifier = self._send(method, params)
        while True:
            event = self._read()
            if event.get("id") == identifier and "method" not in event:
                if "error" in event:
                    raise RuntimeError(event["error"]["message"])
                return event["result"]
            # 请求响应可能与回合通知交错，通知留给当前任务按序消费。
            self._notifications.append(event)

    def _start_thread(self) -> None:
        if self._thread_id is not None:
            return
        self._request("initialize", {"clientInfo": {"name": "actrail-e2e", "version": "1"}})
        self._write({"method": "initialized"})
        result = self._request("thread/start", {"ephemeral": True})
        self._thread_id = result["thread"]["id"]

    def _execute_stream(self, prompt: str) -> AgentResult:
        self._start_thread()
        result = self._request("turn/start", {
            "threadId": self._thread_id, "input": [{"type": "text", "text": prompt}],
        })
        return self._finish_turn(result["turn"]["id"])

    def _execute_compact(self) -> AgentResult:
        self._start_thread()
        self._request("thread/compact/start", {"threadId": self._thread_id})
        return self._finish_turn(None, require_compaction=True)

    def _finish_turn(self, turn_id: str | None, require_compaction: bool = False) -> AgentResult:
        output = []
        compacted = False
        failure = None
        while True:
            event = self._notifications.popleft() if self._notifications else self._read()
            if "id" in event and "method" in event:
                self._write({"id": event["id"], "error": {
                    "code": -32601, "message": "Interactive requests are not supported",
                }})
                failure = "Codex requested interactive approval or input"
                continue
            if "id" in event:
                continue
            params = event.get("params", {})
            if params.get("threadId") != self._thread_id:
                continue
            method = event.get("method")
            if method == "turn/started" and turn_id is None:
                turn_id = params["turn"]["id"]
            elif method == "item/completed" and params.get("turnId") == turn_id:
                item = params["item"]
                if item["type"] == "agentMessage":
                    output.append(item["text"])
                elif item["type"] == "contextCompaction":
                    compacted = True
            elif method == "turn/completed" and params["turn"]["id"] == turn_id:
                turn = params["turn"]
                if turn["status"] != "completed":
                    failure = (turn.get("error") or {}).get("message") or turn["status"]
                if require_compaction and not compacted:
                    failure = failure or "Codex did not perform context compaction"
                return AgentResult("\n".join(output), failure is None, failure)

    def _execute(self, prompt: str) -> AgentResult:
        process = self._session.process
        process.stdin.write(prompt)
        process.stdin.close()
        output = []
        failure = None
        completed = False
        # exec 的 JSON 输出是事件流，turn.completed 与进程退出共同确认完成。
        for line in process.stdout:
            event = json.loads(line)
            kind = event.get("type")
            if kind == "item.completed":
                item = event.get("item", {})
                if item.get("type") == "agent_message":
                    output.append(item.get("text", ""))
            elif kind == "turn.completed":
                completed = True
            elif kind == "turn.failed":
                failure = event.get("error", {}).get("message", "Codex turn failed")
        returncode = process.wait()
        if returncode:
            failure = failure or f"Codex exited with {returncode}"
        elif not completed:
            failure = failure or "Codex exited before completing the turn"
        return AgentResult("\n".join(output), failure is None, failure)

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
