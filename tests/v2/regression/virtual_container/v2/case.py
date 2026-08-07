from __future__ import annotations

from tests.v2.common.core import has_failure, TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import VirtualContainerConfig
from .prerequisites import VirtualContainerPrerequisites
from .scenario import VirtualContainerScenario


class VirtualContainerCase(TestCase):
    def __init__(self, config: VirtualContainerConfig):
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        skipped_backends: list[str] = []
        executed_backends = 0
        prerequisites = VirtualContainerPrerequisites(self._config)
        if not prerequisites.kvm_available():
            return TestResult(
                TestStatus.SKIPPED,
                "readable/writable /dev/kvm is unavailable; "
                "virtual-container acceptance was not run",
            )
        release_problem = prerequisites.release_problem()
        if release_problem is not None:
            return release_problem

        scenario = VirtualContainerScenario(self._config, test_context)
        try:
            test_context.report_progress(
                "contracts",
                "running deployment and no-KVM lifecycle contracts",
            )
            results["contracts"] = scenario.run_contracts()
            if has_failure(results["contracts"]):
                return TestResult(
                    TestStatus.COMPOSITE,
                    "virtual-container V2 acceptance",
                    results,
                )
            if self._config.scope == "contracts":
                return TestResult(
                    TestStatus.SKIPPED,
                    "contracts passed; KVM runtime acceptance was not run",
                    results,
                )
            test_context.report_progress(
                "artifacts",
                "validating release, bundles, images and runtime configs",
            )
            deployment, deployment_problem = prerequisites.resolve_deployment()
            if deployment_problem is not None:
                results["artifact_manifest"] = deployment_problem
                return TestResult(
                    TestStatus.COMPOSITE,
                    "virtual-container V2 acceptance",
                    results,
                )
            if deployment is None:
                results["artifact_manifest"] = TestResult(
                    TestStatus.PASSED,
                    "deprecated mutable paths match the current release",
                )
                test_context.report_progress(
                    "artifacts",
                    "deprecated legacy paths in use; prepare a format 2 profile",
                )
            else:
                results["artifact_manifest"] = TestResult(
                    TestStatus.PASSED,
                    f"content-addressed cache hit {deployment.cache_key}",
                )
            scenario = VirtualContainerScenario(
                self._config,
                test_context,
                deployment,
            )

            test_context.report_progress(
                "backends",
                "resolving each selected backend independently",
            )
            for name, backend in prerequisites.resolve_backends(
                deployment
            ).items():
                if backend.problem is not None:
                    results[name] = backend.problem
                    if backend.problem.status is TestStatus.SKIPPED:
                        skipped_backends.append(name)
                    continue
                executed_backends += 1
                results[name] = scenario.run_backend(backend)
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        acceptance = TestResult(
            TestStatus.COMPOSITE,
            "virtual-container V2 acceptance",
            results,
        )
        if skipped_backends and executed_backends == 0 and not has_failure(acceptance):
            return TestResult(
                TestStatus.SKIPPED,
                "KVM runtime acceptance was not run for: "
                + ", ".join(skipped_backends),
                results,
            )
        return acceptance
