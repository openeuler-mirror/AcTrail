from __future__ import annotations

from tests.e2e.common.core import TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton

from .config import CommandPolicyXiaooConfig
from .environment import CommandPolicyXiaooEnvironment
from .verification.audit import CommandAuditVerification


class CommandPolicyXiaooCase(TestCase):
    def __init__(self, config: CommandPolicyXiaooConfig):
        self._config = config
        self._environment: CommandPolicyXiaooEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "environment_prepare",
                "starting local provider, actraild, actrailweb, and command publisher",
            )
            self._environment = CommandPolicyXiaooEnvironment(
                self._config, test_context.output
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "isolated services and wasm.command-policy-dynamic are active",
            )

            test_context.report_progress(
                "atomic_validation",
                "rejecting an out-of-grant all-or-nothing candidate",
            )
            revision = self._environment.require_atomic_rejection()
            results["atomic_validation"] = TestResult(
                TestStatus.PASSED,
                f"plugin memory and daemon revision {revision} remained unchanged",
            )

            test_context.report_progress(
                "policy_update",
                "publishing and dry-running the Bash deny rule",
            )
            dry_run = self._environment.publish_deny()
            results["policy_update"] = TestResult(
                TestStatus.PASSED,
                "stable command-dynamic-1 route matched: " + dry_run.strip(),
            )

            test_context.report_progress(
                "argv_scope",
                "allowing the same Bash binary outside the configured -c wildcard",
            )
            self._environment.require_nonmatching_args_allowed()
            results["argv_scope"] = TestResult(
                TestStatus.PASSED,
                "Bash --version was allowed while [-c, *] remained governed",
            )

            test_context.report_progress(
                "thread_identity",
                "allowing Bash exec from a non-leader worker thread",
            )
            self._environment.require_nonleader_exec_allowed()
            results["thread_identity"] = TestResult(
                TestStatus.PASSED,
                "non-leader exec resolved the governing process identity",
            )
            errno_trace = self._environment.require_native_lookup_errnos()
            results["native_errno"] = TestResult(
                TestStatus.PASSED, f"trace-{errno_trace} actual exec calls returned ENOENT and ENOTDIR"
            )

            test_context.report_progress(
                "xiaoo_denied",
                "running real Xiaoo Bash tool under command enforcement",
            )
            denied_trace, denied = self._environment.run_xiaoo(
                "v2-command-policy-xiaoo-denied"
            )
            self._environment.require_denied(denied)
            results["xiaoo_denied"] = TestResult(
                TestStatus.PASSED,
                f"trace-{denied_trace} Bash returned EPERM and marker is absent",
            )

            test_context.report_progress(
                "governance_evidence",
                "checking Enforcement and command boundary alert",
            )
            if self._config.audit_enabled:
                _, alert = self._environment.wait_for_evidence(denied_trace)
                results["governance_evidence"] = TestResult(
                    TestStatus.PASSED,
                    f"trace-{denied_trace} has Enforcement and alert-{alert.get('alert_id')}",
                )

            test_context.report_progress(
                "owner_unload",
                "unloading policy owner and running real Xiaoo again",
            )
            self._environment.unload_owner()
            allowed_trace, allowed = self._environment.run_xiaoo(
                "v2-command-policy-xiaoo-owner-unloaded"
            )
            self._environment.require_allowed(allowed)
            results["owner_unload"] = TestResult(
                TestStatus.PASSED,
                f"trace-{allowed_trace} Bash succeeded and created the marker",
            )
            test_context.report_progress(
                "path_search", "checking native lookup errors before governed Bash execution"
            )
            if self._config.audit_enabled:
                self._environment.require_path_search_errors(denied_trace)
                self._environment.require_path_search_errors(allowed_trace)
                self._environment.require_path_search_errors(errno_trace)
            verification = CommandAuditVerification(self._environment)
            for trace_id in (errno_trace, denied_trace, allowed_trace):
                verification.verify(trace_id)
            results["path_search"] = TestResult(
                TestStatus.PASSED,
                "ENOENT/ENOTDIR candidates preserved errno; a nonexecutable candidate allowed continued search; Bash policy deny and owner-unload allow both held",
            )
            results["audit_setting"] = TestResult(
                TestStatus.PASSED,
                f"audit_enabled={self._config.audit_enabled}: actual persisted events and actions match the setting",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "real Xiaoo dynamic command-policy boundary",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "real Xiaoo dynamic command-policy boundary",
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
