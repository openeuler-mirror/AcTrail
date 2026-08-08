"""High-level test orchestration built on the tests.v2.common.core contract.

Regression entry points import the runner API from here; case-authoring
primitives live in tests.v2.common.core.
"""

from .run import (
    TestDefinition,
    add_common_arguments,
    run_one,
    run_selected,
)
from .testing_context import TestingContextSingleton

__all__ = [
    "TestDefinition",
    "TestingContextSingleton",
    "add_common_arguments",
    "run_one",
    "run_selected",
]
