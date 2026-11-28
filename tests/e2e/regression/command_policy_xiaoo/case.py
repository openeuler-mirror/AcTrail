"""Exercise policy decisions through scoped services and reported steps."""
from __future__ import annotations

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton

from .config import CommandPolicyXiaooConfig
from .environment import CommandPolicyXiaooEnvironment
from .verification.audit import CommandAuditVerification


class CommandPolicyXiaooCase(TestCase):
    case_id = 'command_policy_xiaoo'
    description = 'Use Web configuration to deny real Xiaoo Bash execution and verify governance'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = CommandPolicyXiaooConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case('command_policy_xiaoo') as case:
            with CommandPolicyXiaooEnvironment(self._config, context.output) as environment:
                with case.step("Start the test services") as step:
                    step.message = 'starting local provider, actraild, actrailweb, and command publisher'
                    environment.prepare()
                    step.status = TestStatus.PASSED
                    step.message = ("isolated services and wasm.command-policy-dynamic are active")
                with case.step("Reject the entire invalid policy update") as step:
                    step.message = 'rejecting an out-of-grant all-or-nothing candidate'
                    revision = environment.require_atomic_rejection()
                    step.status = TestStatus.PASSED
                    step.message = (f"plugin memory and daemon revision {revision} remained unchanged")
                with case.step("Publish the Bash deny rule") as step:
                    step.message = 'publishing and dry-running the Bash deny rule'
                    dry_run = environment.publish_deny()
                    step.status = TestStatus.PASSED
                    step.message = ("stable command-dynamic-1 route matched: " + dry_run.strip())
                with case.step("Check that unmatched Bash arguments are allowed") as step:
                    step.message = 'allowing the same Bash binary outside the configured -c wildcard'
                    environment.require_nonmatching_args_allowed()
                    step.status = TestStatus.PASSED
                    step.message = ("Bash --version was allowed while [-c, *] remained governed")
                with case.step("Check execution from a worker thread") as step:
                    step.message = 'allowing Bash exec from a non-leader worker thread'
                    environment.require_nonleader_exec_allowed()
                    step.details["thread_identity"] = TestResult(
                        TestStatus.PASSED,
                        "non-leader exec resolved the governing process identity",
                    )
                    errno_trace = environment.require_native_lookup_errnos()
                    step.details["native_errno"] = TestResult(
                        TestStatus.PASSED, f"trace-{errno_trace} actual exec calls returned ENOENT and ENOTDIR"
                    )
                with case.step("Check that policy blocks Bash in XiaoO") as step:
                    step.message = 'running real Xiaoo Bash tool under command enforcement'
                    denied_trace, denied = environment.run_xiaoo(
                        "v2-command-policy-xiaoo-denied"
                    )
                    environment.require_denied(denied)
                    step.status = TestStatus.PASSED
                    step.message = (f"trace-{denied_trace} Bash returned EPERM and marker is absent")
                if self._config.audit_enabled:
                    with case.step("Check the enforcement record and alert") as step:
                        _, alert = environment.wait_for_evidence(denied_trace)
                        step.status = TestStatus.PASSED
                        step.message = (f"trace-{denied_trace} has Enforcement and alert-{alert.get('alert_id')}")
                with case.step("Check that Bash works after unloading the policy owner") as step:
                    step.message = 'unloading policy owner and running real Xiaoo again'
                    environment.unload_owner()
                    allowed_trace, allowed = environment.run_xiaoo(
                        "v2-command-policy-xiaoo-owner-unloaded"
                    )
                    environment.require_allowed(allowed)
                    step.status = TestStatus.PASSED
                    step.message = (f"trace-{allowed_trace} Bash succeeded and created the marker")
                with case.step("Check command lookup errors and policy decisions") as step:
                    step.message = 'checking native lookup errors before governed Bash execution'
                    if self._config.audit_enabled:
                        environment.require_path_search_errors(denied_trace)
                        environment.require_path_search_errors(allowed_trace)
                        environment.require_path_search_errors(errno_trace)
                    verification = CommandAuditVerification(environment)
                    for trace_id in (errno_trace, denied_trace, allowed_trace):
                        verification.verify(trace_id)
                    step.details["path_search"] = TestResult(
                        TestStatus.PASSED,
                        "ENOENT/ENOTDIR candidates preserved errno; a nonexecutable candidate allowed continued search; Bash policy deny and owner-unload allow both held",
                    )
                    step.details["audit_setting"] = TestResult(
                        TestStatus.PASSED,
                        f"audit_enabled={self._config.audit_enabled}: actual persisted events and actions match the setting",
                    )
        return case.result

