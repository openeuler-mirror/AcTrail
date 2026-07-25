from enum import Enum


class TestStatus(Enum):
    SKIPPED = "skipped"
    PASSED = "passed"
    FAILED = "failed"
    COMPOSITE = "composite"  # For composite test cases that contain multiple sub-tests
