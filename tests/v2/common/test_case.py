
from abc import ABC, abstractmethod
from dataclasses import dataclass
from enum import Enum
from tests.v2.common.testing_context import TestingContextSingleton


class TestStatus(Enum):
    SKIPPED = "skipped"
    PASSED = "passed"
    FAILED = "failed"
    COMPOSITE = "composite"  # For composite test cases that contain multiple sub-tests

@dataclass
class TestResult:
    status: TestStatus
    message: str = ""
    details: dict = None  # Optional field to hold additional details about the test result, don't use for non-COMPOSITE test

class TestCase(ABC):
    """
    Abstract base class for test cases.
    """

    @abstractmethod
    def run(self, test_context: TestingContextSingleton) -> TestResult:
        """
        Run the test case.
        """
        pass