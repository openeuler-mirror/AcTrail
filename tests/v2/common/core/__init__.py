"""Base interface for V2 case authoring and test I/O.

This is the dependency-free contract layer: regression cases and the runner
package import these primitives; nothing here imports the runner.
"""

from .config import CommonTestConfig, TestCaseInputs
from .errors import AgentBinaryNotFoundError
from .output import (
    CaseProgressReporter,
    TestOutput,
    effective_status,
    has_failure,
)
from .test_case import TestCase, TestResult, TestStatus

__all__ = [
    "AgentBinaryNotFoundError",
    "CaseProgressReporter",
    "CommonTestConfig",
    "TestCase",
    "TestCaseInputs",
    "TestOutput",
    "TestResult",
    "TestStatus",
    "effective_status",
    "has_failure",
]
