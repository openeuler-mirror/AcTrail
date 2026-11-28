# 用例与步骤上下文：集中报告进度、记录结果并控制失败传播。
from __future__ import annotations

from contextlib import contextmanager
from typing import Iterator, NoReturn

from ..core import TestResult, TestStatus


class _CaseSkipped(Exception):
    """将跳过原因传递至用例边界。"""


class CaseScope:
    """维护结果树；步骤默认传播失败，用例边界负责收束异常。"""

    def __init__(self, name: str):
        self.result = TestResult(TestStatus.COMPOSITE, name)
        self._parents = [self.result]
        # 只记住正在向外传播的异常，避免每层重复添加同一失败。
        self._propagating_failure: tuple[Exception, TestResult] | None = None

    def __enter__(self) -> CaseScope:
        self._report(self.result.message, "started")
        return self

    def __exit__(self, exception_type, exception, traceback) -> bool:
        handled = isinstance(exception, Exception)
        if isinstance(exception, _CaseSkipped):
            self.result.message = str(exception)
            if not self.result.has_failure():
                self.result.status = TestStatus.SKIPPED
        elif handled:
            self._record_failure(self.result, exception)
        self._propagating_failure = None
        if exception is None or handled:
            self._report(self.result.message, self.result.effective_status().value)
        return handled

    def skip(self, message: str) -> NoReturn:
        """终止当前用例；已经记录的失败仍保留在汇总中。"""
        raise _CaseSkipped(message)

    @contextmanager
    def step(self, name: str, propagate: bool = True) -> Iterator[TestResult]:
        """嵌套步骤形成结果树；禁止传播时仅结束当前步骤。"""
        parent = self._parents[-1]
        if name in parent.details:
            raise ValueError(f"duplicate step name: {name}")
        result = TestResult(TestStatus.COMPOSITE)
        parent.details[name] = result
        self._parents.append(result)
        self._report(name, "started")
        try:
            yield result
        except _CaseSkipped as error:
            if not result.has_failure():
                result.status = TestStatus.SKIPPED
            result.message = str(error)
            self._report(name, result.effective_status().value)
            raise
        except Exception as error:
            self._record_failure(result, error)
            self._report(name, result.effective_status().value)
            if propagate:
                raise
            self._propagating_failure = None
        else:
            self._report(name, result.effective_status().value)
        finally:
            self._parents.pop()

    def _record_failure(self, result: TestResult, error: Exception) -> None:
        message = str(error) or type(error).__name__
        notes = getattr(error, "__notes__", ())
        if notes:
            message += "\n" + "\n".join(notes)
        if self._propagating_failure is not None:
            recorded_error, failure = self._propagating_failure
            if error is recorded_error:
                # 资源退出时可能补充清理失败说明，更新同一条失败即可。
                failure.message = message
                return
        # 保留已完成的子步骤；当前作用域自身的失败作为独立结果。
        key = "failure"
        while key in result.details:
            key = "_" + key
        failure = TestResult(TestStatus.FAILED, message)
        result.details[key] = failure
        self._propagating_failure = (error, failure)

    @staticmethod
    def _report(name: str, message: str) -> None:
        from .testing_context import TestingContextSingleton

        TestingContextSingleton.current().report_progress(name, message)
