"""Discover registered regression classes and construct selected scenarios."""
from importlib import import_module
from pathlib import Path

from ..core import TestCase, TestRunDirectories


class TestCaseFactory:
    @classmethod
    def discover(cls, directory: Path) -> list[type[TestCase]]:
        """Import public case modules, then order prerequisites before consumers."""
        for path in sorted(directory.glob('*/case.py')):
            import_module(f'tests.e2e.regression.{path.parent.name}.case')
        registered = TestCase.get_all_test_case_classes()
        ordered = []
        visiting = set()
        visited = set()

        def visit(case_id):
            if case_id in visited:
                return
            if case_id in visiting:
                raise ValueError(f'cyclic test prerequisite: {case_id}')
            visiting.add(case_id)
            case = registered[case_id]
            for prerequisite in case.skip_if_skipped:
                visit(prerequisite)
            visiting.remove(case_id)
            visited.add(case_id)
            ordered.append(case)

        for case_id in sorted(registered):
            visit(case_id)
        return ordered

    @classmethod
    def create(cls, case_id: str, directories: TestRunDirectories) -> TestCase:
        """Resolve the authoritative registry at construction time."""
        return TestCase.get_test_case_class_by_name(case_id)(directories)
