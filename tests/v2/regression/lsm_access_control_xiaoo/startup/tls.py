"""Exercise a TLS-only BPF profile with a real Agent over local HTTPS."""

import json
import re
import time

from scripts.bench.payload.agent import AgentWorkload
from tests.v2.common.core import TestResult, TestStatus
from .reuse import MeasuredRuntime


class TlsStartupScenario:
    def __init__(self, boundaries):
        self.boundaries = boundaries
        self.case = boundaries.case

    def run(self, context, results, patch):
        case = self.case
        self.boundaries._write_patch(patch, observation=False)
        text = patch.read_text().replace('["resource-metrics"]', '["tls-plaintext-payload"]')
        text = text.replace('[payload.tls]\nenabled = false', '[payload.tls]\nenabled = true\ncapture_backend = "bpf-copy"')
        text += '\n[semantic_retention.l4_payload]\nenabled = true\nstats = true\nbody_content = "retained"\n'
        patch.write_text(text)
        runtime = self.boundaries._runtime(MeasuredRuntime, context, patch)
        workload = AgentWorkload(case._config.repo, case._work / "https-maas", case._xiaoo,
                                 turns=2, https=True, input_bytes=1024, timeout_seconds=case._timeout)
        try:
            workload.start()
            runtime.prepare()
            initial = self.boundaries._snapshot()
            if not initial.ids["prog"]:
                raise AssertionError("TLS-only profile did not load BPF programs")
            for index in range(2):
                directory = case._work / f"https-agent-{index}"
                workload.prepare(directory)
                workload.reset()
                environment = self.boundaries._environment()
                environment.update(workload.env)
                launched = runtime.run(runtime.control_command(
                    "launch", "--host-ebpf", "required", "--seccomp-notify", "disabled",
                    "--", *workload.command(directory),
                ), environment=environment, cwd=directory)
                if launched.returncode:
                    raise AssertionError(f"TLS Agent launch failed: {launched.output}")
                output = directory / "output.log"
                output.write_text(launched.output)
                workload.validate(directory, output)
                match = re.search(r"trace trace-(\d+) entered Active", launched.output)
                if match is None:
                    raise AssertionError("TLS Agent trace did not become Active")
                trace = match.group(1)
                deadline = time.monotonic() + case._timeout
                while time.monotonic() < deadline:
                    response = runtime.run_checked(runtime.viewer_command(
                        "payloads", "--trace-id", trace, "--output-format", "json",
                    ), echo=False)
                    segments = json.loads(response.stdout)["payloads"]
                    directions = {segment["direction"] for segment in segments
                                  if segment["trace_id_raw"] == int(trace)
                                  and segment["source_boundary"] == "TlsUserSpace"
                                  and segment["content_state"] == "Plaintext"
                                  and segment["captured_size"] > 0}
                    if {"outbound", "inbound"} <= directions:
                        break
                    time.sleep(case._poll)
                else:
                    raise AssertionError("TLS-only profile lost HTTPS plaintext observations")
                current = self.boundaries._snapshot()
                for kind in ("prog", "map"):
                    if current.ids[kind] != initial.ids[kind]:
                        raise AssertionError(f"TLS launch replaced shared {kind} resources")
            log = self.boundaries._log()
            for marker in ("host_ebpf_preparation started", "ebpf_runtime_load started"):
                if log.count(marker) != 1:
                    raise AssertionError(f"TLS-only runtime repeated {marker}")
            self.boundaries._stop(runtime)
            initial.require_released(case._timeout, case._poll)
            results["tls_only"] = TestResult(TestStatus.PASSED, "two real HTTPS Xiaoo runs captured bidirectional TLS plaintext with one BPF preparation")
        finally:
            workload.stop()
