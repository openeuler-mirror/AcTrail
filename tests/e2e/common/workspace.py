"""Provide the current working directory through nested workspace contexts."""
from __future__ import annotations

from contextvars import ContextVar
from contextlib import contextmanager
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import ClassVar, Self


class Workspace:
    """指定目录由调用者拥有；随机目录由本作用域创建并清理。"""

    _current: ClassVar[ContextVar[Workspace | None]] = ContextVar("workspace", default=None)

    def __init__(self, path: Path | None = None):
        self.path = path.resolve() if path is not None else None
        self._directory: TemporaryDirectory | None = None

    @classmethod
    def current(cls) -> Workspace | None:
        """取得当前上下文的 Workspace；尚未进入作用域时返回 None。"""
        return cls._current.get()

    @classmethod
    @contextmanager
    def scope(cls, path: Path | None = None):
        """显式目录开启嵌套作用域；未指定时借用当前目录或创建临时目录。"""
        current = cls.current()
        if path is None and current is not None:
            yield current
        else:
            with cls(path) as workspace:
                yield workspace

    def __enter__(self) -> Self:
        if self.path is None:
            self._directory = TemporaryDirectory(prefix="actrail-agent-")
            self.path = Path(self._directory.name).resolve()
        self._token = self._current.set(self)
        return self

    def __exit__(self, exception_type, exception, traceback) -> None:
        # 先恢复外层上下文，目录删除失败也不会遗留当前绑定。
        self._current.reset(self._token)
        if self._directory is not None:
            try:
                self._directory.cleanup()
            except Exception as error:
                if exception is None:
                    raise
                exception.add_note(f"Workspace cleanup failed: {error}")
            finally:
                self._directory = None
                self.path = None
