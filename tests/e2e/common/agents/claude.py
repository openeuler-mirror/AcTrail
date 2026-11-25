"""Drive Claude through its headless text and stream-json protocols."""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult, ToolAccess
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from .process import AgentProcess


class ClaudeAgent(Agent):
    """Translate Claude input and result events while sharing process ownership."""

    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """Claude 模型选择遵循测试配置、Anthropic 配置和已有默认值。"""
        return os.environ.get("CLAUDE_E2E_MODEL") or os.environ.get("ANTHROPIC_MODEL") or MODEL_DEFAULTS.claude_model

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("CLAUDE_E2E_BINARY", "claude")
        if self._binary is None:
            raise FileNotFoundError("Claude executable not found")
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
            arguments.remove(multiple=("--tools", "--allowedTools", "--allowed-tools",
                                       "--disallowedTools", "--disallowed-tools"))
            tools = {ToolAccess.NONE: "", ToolAccess.READ_ONLY: "Read,Glob,Grep",
                     ToolAccess.WRITABLE: "default"}[options.tool_access]
            arguments.append("--tools", tools)
        if options.max_turns is not None:
            arguments.remove(values=("--max-turns",))
            arguments.append("--max-turns", str(options.max_turns))
        return arguments.values

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        self._stream_mode = stream_mode
        launch_process(self._command(stream_mode, options), environment=self._environment, cwd=self._cwd)
        return self

    def _command(self, stream_mode: bool, options: LaunchOptions | None = None) -> list[Path | str]:
        """构造 headless 命令，两种输入模式均使用 JSON 流报告执行结果。"""
        arguments = self._configured_arguments(options)
        model = self._environment.get("CLAUDE_E2E_MODEL")
        if model and not any(arg == "--model" or arg.startswith("--model=") for arg in arguments):
            arguments = (*arguments, "--model", model)
        # --verbose 提供流式事件；--no-session-persistence 避免测试会话落盘。
        # --safe-mode 关闭用户自定义扩展，工具范围由本次启动选项决定。
        return [
            self._binary,
            *arguments,
            "--print",
            "--input-format", "stream-json" if stream_mode else "text",
            "--output-format", "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--safe-mode",
        ]

    def _execute(self, prompt: str, require_compaction: bool = False) -> AgentResult:
        """提交一条输入，读取该轮的结构化结果并保留失败信息。"""
        process = self._session.process
        if process is None or process.stdin is None or process.stdout is None:
            raise RuntimeError("Claude has not been launched")
        if self._stream_mode:
            # 每条输入占一行 JSON；保持 stdin 打开，以便同一进程继续下一轮。
            message = {
                "type": "user",
                "message": {"role": "user", "content": prompt},
                "parent_tool_use_id": None,
            }
            process.stdin.write(json.dumps(message, ensure_ascii=False) + "\n")
            process.stdin.flush()
        else:
            # text 模式读取完整 stdin；关闭写端，用 EOF 标记本次输入结束。
            process.stdin.write(prompt)
            process.stdin.close()

        # 受观测启动的前缀由 Runtime 消费，两种启动路径均在此读取 Claude 协议。
        # assistant 和工具事件可能出现多次，只有 result 表示这一轮结束。
        compacted = False
        for line in process.stdout:
            event = json.loads(line)
            # 压缩证据只属于当前任务，原始协议留在 Claude 适配器内部。
            if (
                event.get("type") == "system"
                and event.get("subtype") == "compact_boundary"
                and event.get("compact_metadata", {}).get("trigger") == "manual"
            ):
                compacted = True
            if event.get("type") != "result":
                continue
            output = event.get("result") or ""
            # 正常收到的失败结果交给调用方判断；结束事件本身不代表成功。
            succeeded = event.get("subtype") == "success" and not event.get("is_error", False)
            errors = event.get("errors") or []
            error = None if succeeded else "\n".join(errors) or output or event.get("subtype")
            if succeeded and require_compaction and not compacted:
                return AgentResult(output, False, "Claude did not perform context compaction")
            if not self._stream_mode:
                process.stdout.read()
                code = process.wait()
                if code:
                    succeeded = False
                    error = error or f"Claude exited with {code}"
            return AgentResult(output=output, succeeded=succeeded, error=error)
        # 未收到 result 就遇到 EOF，属于协议中断，不能据此认定任务完成。
        raise RuntimeError("Claude exited before emitting a result message")

    def run(self, prompt: str) -> None:
        self._session.submit(lambda: self._execute(prompt), single_task=not self._stream_mode)

    def compact(self) -> None:
        if not self._stream_mode:
            raise NotImplementedError("Context compaction requires a streaming Claude session")
        self._session.submit(lambda: self._execute("/compact", require_compaction=True))

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
