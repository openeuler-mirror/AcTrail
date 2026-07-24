#!/usr/bin/env python3

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.test_case import TestResult, TestStatus  # noqa: E402
from tests.v2.common.testing_context import TestingContextSingleton  # noqa: E402
from tests.v2.regression.probe_codex_llm.case import ProbeCodexLLMCase  # noqa: E402
from tests.v2.regression.probe_codex_llm.config import (  # noqa: E402
    ProbeCodexLLMConfig,
)


class ProbeCodexLLMRunner:
    def run(self) -> int:
        if os.geteuid() != 0:
            print("failed: this eBPF E2E must run as root")
            return 1
        arguments = self._arguments()
        config = ProbeCodexLLMConfig.from_environment(REPO, arguments.bin_dir)
        result = ProbeCodexLLMCase(config).run(TestingContextSingleton())
        self._print_result("probe_codex_llm", result, 0)
        return 1 if self._has_failure(result) else 0

    def _arguments(self) -> argparse.Namespace:
        parser = argparse.ArgumentParser(
            description="Run Codex through nested actrailctl launch and verify LLM capture"
        )
        parser.add_argument(
            "--bin-dir",
            type=Path,
            default=Path("target/release"),
            help="directory containing release Actrail binaries",
        )
        return parser.parse_args()

    def _print_result(self, name: str, result: TestResult, depth: int) -> None:
        indent = "  " * depth
        print(f"{indent}{name}: {result.status.value}: {result.message}")
        if result.status == TestStatus.COMPOSITE and result.details:
            for child_name, child in result.details.items():
                self._print_result(child_name, child, depth + 1)

    def _has_failure(self, result: TestResult) -> bool:
        if result.status == TestStatus.FAILED:
            return True
        if result.status != TestStatus.COMPOSITE or not result.details:
            return False
        return any(self._has_failure(child) for child in result.details.values())


if __name__ == "__main__":
    raise SystemExit(ProbeCodexLLMRunner().run())
