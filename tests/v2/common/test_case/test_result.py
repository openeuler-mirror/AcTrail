from __future__ import annotations

from dataclasses import dataclass, field

from .test_status import TestStatus


@dataclass
class TestResult:
    status: TestStatus
    message: str = ""
    details: dict[str, "TestResult"] = field(default_factory=dict)
