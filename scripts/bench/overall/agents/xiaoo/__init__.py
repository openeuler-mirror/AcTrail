"""xiaoo agent backend for the overall replay benchmark."""

from __future__ import annotations

from pathlib import Path

from .. import AgentBackend, resolve_binary


DEFAULT_MODEL = "deepseek-v4-flash"
DEFAULT_TOOLS = "file_read,glob,grep"
DEFAULT_PROMPT = (
    "只读分析任务：请用 file_read/glob/grep 分析 "
    "/home/yzh/projects/AcTrail/tests/v2/common 的结构，"
    "然后输出一份总结。禁止修改任何文件。"
)
def build_command_with_binary(
    binary: Path | None,
    replay_port: int,
    prompt: str,
    max_turns: int,
) -> list[str]:
    if binary is None:
        raise SystemExit("xiaoo not found; pass --xiaoo explicitly")
    return [
        str(binary),
        "--cli",
        "run",
        "-p",
        prompt,
        "--provider",
        "openai",
        "--api-base",
        f"http://127.0.0.1:{replay_port}",
        "--api-key",
        "bench",
        "--model",
        DEFAULT_MODEL,
        "--max-turns",
        str(max_turns),
        "--tools",
        DEFAULT_TOOLS,
        "--debug",
    ]


def backend(configured_binary: str | None) -> AgentBackend:
    binary = resolve_binary(configured_binary, "xiaoo")
    return AgentBackend(
        name="xiaoo",
        binary=binary,
        command=lambda port, prompt, turns: build_command_with_binary(
            binary,
            port,
            prompt,
            turns,
        ),
    )
