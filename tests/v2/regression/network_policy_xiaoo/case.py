from __future__ import annotations

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import NetworkPolicyXiaooConfig
from .environment import NetworkPolicyXiaooEnvironment


class NetworkPolicyXiaooCase(TestCase):
    def __init__(self, config: NetworkPolicyXiaooConfig):
        self._config = config
        self._environment: NetworkPolicyXiaooEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "environment_prepare",
                "starting local provider, actraild, actrailweb, and network publisher",
            )
            self._environment = NetworkPolicyXiaooEnvironment(
                self._config, test_context.output
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "isolated services and wasm.network-policy-dynamic are active",
            )

            test_context.report_progress(
                "xiaoo_allowed_baseline",
                "running real Xiaoo against the empty default-allow route",
            )
            initial_revision = self._environment.require_default_route()
            baseline_trace, baseline = self._environment.run_xiaoo(
                "v2-network-policy-xiaoo-allowed-baseline"
            )
            self._environment.require_allowed(baseline)
            results["xiaoo_allowed_baseline"] = TestResult(
                TestStatus.PASSED,
                f"trace-{baseline_trace} completed the provider and Bash tool round trip",
            )

            test_context.report_progress(
                "policy_update",
                "publishing an exact-endpoint deny rule and dry-running the provider",
            )
            dry_run, rule_revision, source_revision = self._environment.publish_deny(
                initial_revision
            )
            results["policy_update"] = TestResult(
                TestStatus.PASSED,
                f"provider route matched the exact endpoint rule: {dry_run.strip()}",
            )

            test_context.report_progress(
                "xiaoo_denied",
                "running real Xiaoo with its provider connect denied",
            )
            denied_trace, denied = self._environment.run_xiaoo(
                "v2-network-policy-xiaoo-denied"
            )
            self._environment.require_denied(denied)
            results["xiaoo_denied"] = TestResult(
                TestStatus.PASSED,
                f"trace-{denied_trace} reported the governed provider-connect failure",
            )

            test_context.report_progress(
                "governance_evidence",
                "checking network decision attribution in SQLite",
            )
            metadata = self._environment.wait_for_evidence(
                denied_trace,
                rule_revision,
            )
            results["governance_evidence"] = TestResult(
                TestStatus.PASSED,
                f"trace-{denied_trace} attributed EPERM to {metadata.get('rule_id')}",
            )

            test_context.report_progress(
                "policy_recovery",
                "clearing the deny rule and running real Xiaoo again",
            )
            restored_revision = self._environment.clear_deny(source_revision)
            restored_trace, restored = self._environment.run_xiaoo(
                "v2-network-policy-xiaoo-restored"
            )
            self._environment.require_allowed(restored)
            results["policy_recovery"] = TestResult(
                TestStatus.PASSED,
                f"trace-{restored_trace} succeeded after source revision "
                f"advanced to {restored_revision}",
            )

            return TestResult(
                TestStatus.COMPOSITE,
                "real Xiaoo dynamic network-policy boundary",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "real Xiaoo dynamic network-policy boundary",
                results,
            )

    def cleanup(
        self,
        test_context: TestingContextSingleton,
    ) -> TestResult | None:
        del test_context
        if self._environment is None:
            return None
        return self._environment.cleanup()
