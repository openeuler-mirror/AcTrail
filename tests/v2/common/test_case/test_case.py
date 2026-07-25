from __future__ import annotations

from abc import ABC, ABCMeta, abstractmethod
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tests.v2.common.testing_context import TestingContextSingleton

from .test_result import TestResult


class TestCaseMeta(ABCMeta):
    """
    Metaclass for TestCase to ensure that all subclasses implement the run method.
    """

    __name2cls__ = dict()

    def __new__(mcs, name, bases, namespace):
        cls = super().__new__(mcs, name, bases, namespace)
        if not cls.__abstractmethods__:
            mcs.__name2cls__[name] = cls
        return cls

    def get_test_case_class_by_name(cls, name: str) -> type[TestCase]:
        """
        Get the test case class by its name.
        """
        if name not in cls.__name2cls__:
            raise ValueError(f"unknown test case class: {name}")
        return cls.__name2cls__[name]

    def get_all_test_case_classes(cls) -> dict[str, type[TestCase]]:
        """
        Get all registered test case classes.
        """
        return dict(cls.__name2cls__)


class TestCase(ABC, metaclass=TestCaseMeta):
    """
    Abstract base class for test cases.
    """

    @abstractmethod
    def run(self, test_context: TestingContextSingleton) -> TestResult:
        """
        Run the test case.
        """
        raise NotImplementedError
