"""High-level test orchestration built on the tests.e2e.common.core contract.

Regression entry points import the runner API from here; case-authoring
primitives live in tests.e2e.common.core.
"""

from .run import (
    add_common_arguments,
    run_one,
    run_selected,
)
from .testing_context import TestingContextSingleton

__all__ = [
    "TestingContextSingleton",
    "TestCaseFactory",
    "add_common_arguments",
    "run_one",
    "run_selected",
]

from .factory import TestCaseFactory
