"""Encapsulate Qoder CLI turns and compaction in its own adapter."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult, ToolAccess
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess


class QoderCliAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """读取 Qoder 用户设置中的默认模型。"""
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("QODERCLI_E2E_BINARY", "qodercli")
        if selected is None:
            return None
        environment = discovery.environment(selected)
        path = Path(environment["HOME"]) / ".qoder" / "settings.json"
        try:
            model = json.loads(path.read_text(encoding="utf-8"))["model"]["name"]
        except (OSError, json.JSONDecodeError, KeyError, TypeError):
            return None
        return model if isinstance(model, str) and model else None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("QODERCLI_E2E_BINARY", "qodercli")
        if self._binary is None:
            raise FileNotFoundError("Qoder CLI executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)
        self._stream_mode = True

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _configured_arguments(self, options: LaunchOptions | None) -> list[str]:
        arguments = AgentArguments(self._arguments)
        if options is None:
            return arguments.values
        if options.tool_access is not None:
            arguments.remove(multiple=("--tools",), values=("--allowed-tools", "--disallowed-tools"))
            tools = {ToolAccess.NONE: "", ToolAccess.READ_ONLY: "Read,Glob,Grep",
                     ToolAccess.WRITABLE: "default"}[options.tool_access]
            arguments.append("--tools", tools)
        return arguments.values

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        arguments = self._configured_arguments(options)
        self._stream_mode = stream_mode
        launch_process(
            [self._binary, *arguments, "--print", "--no-session-persistence",
             "--input-format", "stream-json" if stream_mode else "text",
             "--output-format", "stream-json"],
            environment=self._environment, cwd=self._cwd,
        )
        return self

    def run(self, prompt: str) -> None:
        self._session.submit(lambda: self._execute(prompt), single_task=not self._stream_mode)

    def compact(self) -> None:
        if not self._stream_mode:
            raise NotImplementedError("Qoder CLI compaction requires a streaming session")
        self._session.submit(lambda: self._execute("/compact", require_compaction=True))

    def _execute(self, prompt: str, require_compaction: bool = False) -> AgentResult:
        process = self._session.process
        if self._stream_mode:
            process.stdin.write(json.dumps({
                "type": "user", "message": {"role": "user", "content": prompt},
                "parent_tool_use_id": None,
            }) + "\n")
            process.stdin.flush()
        else:
            process.stdin.write(prompt)
            process.stdin.close()
        compacted = False
        for line in process.stdout:
            event = json.loads(line)
            # 压缩证据和结果格式完全留在适配器内。
            if (event.get("type") == "system" and event.get("subtype") == "compact_boundary"
                    and event.get("compact_metadata", {}).get("trigger") == "manual"):
                compacted = True
            if event.get("type") != "result":
                continue
            output = event.get("result") or ""
            success = event.get("subtype") == "success" and not event.get("is_error", False)
            errors = event.get("errors") or []
            error = None if success else "\n".join(errors) or output or event.get("subtype") or "Qoder CLI task failed"
            if require_compaction and success and not compacted:
                error = "Qoder CLI did not perform context compaction"
            if not self._stream_mode:
                process.stdout.read()
                code = process.wait()
                if code:
                    error = error or f"Qoder CLI exited with {code}"
            return AgentResult(output, error is None, error)
        raise RuntimeError("Qoder CLI exited before emitting a result")

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
