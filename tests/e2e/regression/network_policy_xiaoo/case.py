"""Exercise policy decisions through scoped services and reported steps."""
from __future__ import annotations

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton

from .config import NetworkPolicyXiaooConfig
from .environment import NetworkPolicyXiaooEnvironment


class NetworkPolicyXiaooCase(TestCase):
    case_id = 'network_policy_xiaoo'
    description = 'Toggle an exact-endpoint Web rule around real Xiaoo and verify attribution'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = NetworkPolicyXiaooConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case('network_policy_xiaoo') as case:
            with NetworkPolicyXiaooEnvironment(self._config, context.output) as environment:
                with case.step("Start the test services") as step:
                    step.message = 'starting local provider, actraild, actrailweb, and network publisher'
                    environment.prepare()
                    step.status = TestStatus.PASSED
                    step.message = ("isolated services and wasm.network-policy-dynamic are active")
                with case.step("Check that XiaoO can connect before the deny rule") as step:
                    step.message = 'running real Xiaoo against the empty default-allow route'
                    initial_revision = environment.require_default_route()
                    baseline_trace, baseline = environment.run_xiaoo(
                        "v2-network-policy-xiaoo-allowed-baseline"
                    )
                    environment.require_allowed(baseline)
                    step.status = TestStatus.PASSED
                    step.message = (f"trace-{baseline_trace} completed the provider and Bash tool round trip")
                with case.step("Publish the provider endpoint deny rule") as step:
                    step.message = 'publishing an exact-endpoint deny rule and dry-running the provider'
                    dry_run, rule_revision, source_revision = environment.publish_deny(
                        initial_revision
                    )
                    step.status = TestStatus.PASSED
                    step.message = (f"provider route matched the exact endpoint rule: {dry_run.strip()}")
                with case.step("Check that XiaoO cannot connect to the provider") as step:
                    step.message = 'running real Xiaoo with its provider connect denied'
                    denied_trace, denied = environment.run_xiaoo(
                        "v2-network-policy-xiaoo-denied"
                    )
                    environment.require_denied(denied)
                    step.status = TestStatus.PASSED
                    step.message = (f"trace-{denied_trace} reported the governed provider-connect failure")
                with case.step("Check the recorded network decision") as step:
                    step.message = 'checking network decision attribution in SQLite'
                    metadata = environment.wait_for_evidence(
                        denied_trace,
                        rule_revision,
                    )
                    step.details["governance_evidence"] = TestResult(
                        TestStatus.PASSED,
                        f"trace-{denied_trace} attributed EPERM to {metadata.get('rule_id')}",
                    )
                    alerts = environment.wait_for_boundary_alerts(denied_trace)
                    if environment.api.alerts(baseline_trace).get("alerts"):
                        raise AssertionError("allowed baseline generated a boundary alert")
                    step.details["boundary_alerts"] = TestResult(
                        TestStatus.PASSED,
                        f"trace-{denied_trace}: {len(alerts)} network boundary alerts verified in SQLite and Web",
                    )
                with case.step("Check that XiaoO can connect after clearing the rule") as step:
                    step.message = 'clearing the deny rule and running real Xiaoo again'
                    restored_revision = environment.clear_deny(source_revision)
                    restored_trace, restored = environment.run_xiaoo(
                        "v2-network-policy-xiaoo-restored"
                    )
                    environment.require_allowed(restored)
                    step.details["policy_recovery"] = TestResult(
                        TestStatus.PASSED,
                        f"trace-{restored_trace} succeeded after source revision "
                        f"advanced to {restored_revision}",
                    )
                    environment.finish_boundary_assertions((baseline_trace, restored_trace), denied_trace)
                    step.details["allowed_alerts"] = TestResult(
                        TestStatus.PASSED,
                        "network denial deltas match received events; allowed and recovered traces have no network alerts after queue drain",
                    )
        return case.result

