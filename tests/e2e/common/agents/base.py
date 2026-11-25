"""Define the abstract agent contract and register concrete adapters."""

from __future__ import annotations

from abc import ABC, ABCMeta, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import ClassVar, Self, TYPE_CHECKING

if TYPE_CHECKING:
    from tests.e2e.common.actrail_runtime import LaunchResult


class AgentArguments:
    """Replace adapter-owned CLI options without changing unrelated arguments."""

    def __init__(self, arguments: tuple[str, ...]):
        self.values = list(arguments)

    def remove(self, *, values: tuple[str, ...] = (), switches: tuple[str, ...] = (),
               multiple: tuple[str, ...] = ()) -> None:
        remaining = []
        index = 0
        while index < len(self.values):
            argument = self.values[index]
            flag, separator, _ = argument.partition("=")
            index += 1
            if flag in values:
                if not separator:
                    index += 1
            elif flag in multiple:
                if not separator:
                    while index < len(self.values) and not self.values[index].startswith("-"):
                        index += 1
            elif flag not in switches:
                remaining.append(argument)
        self.values = remaining

    def append(self, *arguments: str) -> None:
        self.values.extend(arguments)

@dataclass(frozen=True)
class AgentResult:
    """Outcome of one submitted task."""

    output: str
    succeeded: bool
    error: str | None = None


class _AgentMeta(ABCMeta):
    """Register imported concrete classes by name, e.g. XiaoOAgent as xiaoo."""

    registry: ClassVar[dict[str, type[Agent]]] = {}

    def __new__(metaclass, name, bases, namespace, **kwargs):
        agent_class = super().__new__(metaclass, name, bases, namespace, **kwargs)
        if not agent_class.__abstractmethods__:
            key = name.removesuffix("Agent").lower()
            if key in metaclass.registry:
                raise ValueError(f"Agent already registered: {key}")
            metaclass.registry[key] = agent_class
        return agent_class


class Agent(ABC, metaclass=_AgentMeta):
    """Define agent operations while adapters own execution and protocol state."""

    _binary: Path
    _environment: dict[str, str]

    @property
    def kind(self) -> str:
        """Registered adapter name used by the factory."""
        return type(self).__name__.removesuffix("Agent").lower()

    @property
    def binary(self) -> Path:
        """Executable selected for this instance."""
        return self._binary

    @property
    def environment(self) -> dict[str, str]:
        """Environment passed to the Agent process."""
        return dict(self._environment)

    @classmethod
    @abstractmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """查询该 Agent 的默认模型；无法确定或未覆盖 CLI 默认时返回 None。"""
        raise NotImplementedError

    @abstractmethod
    def launch(self, stream_mode: bool = True) -> Self:
        """直接启动 Agent，使用与受观测启动相同的目录和退出生命周期。"""
        raise NotImplementedError

    @abstractmethod
    def launchWithAcTrailCtl(
        self, stream_mode: bool = True
    ) -> Self:
        """Launch through the current runtime context and return this agent."""
        raise NotImplementedError

    @abstractmethod
    def run(self, prompt: str) -> None:
        """Submit a task using the agent's protocol."""
        raise NotImplementedError

    def compact(self) -> None:
        """提交上下文压缩任务；不支持该能力的适配器使用默认实现。"""
        raise NotImplementedError("Context compaction is not supported")

    @abstractmethod
    def join_all(self, timeout: float | None, **kwargs) -> None:
        """Wait for submitted tasks to finish, irrespective of their success."""
        raise NotImplementedError

    @property
    @abstractmethod
    def results(self) -> list[AgentResult]:
        """按提交顺序返回已完成任务的结果。"""
        raise NotImplementedError

    @property
    @abstractmethod
    def launch_result(self) -> LaunchResult | None:
        """受观测启动的结果；直接启动或尚未启动时为 None。"""
        raise NotImplementedError

    @property
    @abstractmethod
    def returncode(self) -> int | None:
        """进程退出码；尚未启动或尚未退出时为 None。"""
        raise NotImplementedError

    @property
    @abstractmethod
    def diagnostics(self) -> str:
        """CLI 诊断输出；退出作用域后内容完整。"""
        raise NotImplementedError

    @property
    @abstractmethod
    def command(self) -> tuple[str, ...]:
        """实际启动命令；尚未启动时为空。"""
        raise NotImplementedError

    def __enter__(self) -> Self:
        """Return the agent prepared by launchWithAcTrailCtl."""
        return self

    @abstractmethod
    def __exit__(
        self,
        exception_type: type[BaseException] | None,
        exception: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        """Release resources owned by the concrete agent."""
        raise NotImplementedError

