from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class AgentSpec:
    name: str
    binary: Path
    environment: dict[str, str]
    model: dict[str, str] | None
    arguments: tuple[str, ...] = ()


@dataclass(frozen=True)
class ConversationScenario:
    name: str
    prompts: tuple[str, ...]
    compact_turn_index: int


@dataclass(frozen=True)
class AgentRun:
    trace_id: int


SLEEP_PROMPT = (
    "请使用 bash 执行 sleep 0.1，结束后用 printf 输出 SLEEP_SECONDS=0.1。"
    "只执行这一次 bash 工具调用，完成后只回复 SLEEP_COMPLETED。"
)
RECALL_PROMPT = (
    "请从当前会话摘要中直接读取已完成 sleep 命令记录的时长，无需重新执行、验证或推导。"
    "不要调用工具，不要解释或展开分析。"
    "只回复一行 SLEEP_SECONDS=<记录的秒数>，不要使用 Markdown 或添加其他文字。"
)

SCENARIOS = (
    ConversationScenario(
        name="compact",
        prompts=(SLEEP_PROMPT, "/compact", RECALL_PROMPT),
        compact_turn_index=1,
    ),
)
