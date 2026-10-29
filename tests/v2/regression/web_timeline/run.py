#!/usr/bin/env python3
"""Generate real agent evidence with fresh isolated defaults, then test the Web UI."""
from constants import CASE_CONSTANTS as C
import argparse
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import time
from urllib.request import urlopen
from content import CapturedRequest
from long_content.capture import LongContentCapture

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput
from tests.v2.common.patch_builder import PatchBuilder


class Acceptance:
    def __init__(self, args):
        self.args = args
        self.work = args.work.resolve()
        self.work.mkdir(parents=True, exist_ok=False)
        self.runtime = ActrailRuntime.isolated(
            REPO, args.bin_dir.resolve(), C.command_timeout_seconds, TestOutput(color_mode="never"),
            self.work, agent_invocation_commands=[args.agent.resolve()])
        self.web = None

    def start_web(self, name, *, config=None, **settings):
        if self.web:
            self.web.terminate()
            self.web.wait(timeout=C.web_stop_timeout_seconds)
        with (self.work / f'{name}.log').open('w') as log:
            self.web = subprocess.Popen([
                str(self.args.bin_dir.resolve() / 'actrailweb'), '--config', str(config or self.work / 'actraild.conf'),
                '--addr', C.host, '--port', str(self.args.port)],
                stdout=log, stderr=subprocess.STDOUT, cwd=REPO, env=dict(os.environ, **settings))
        base = f'http://{C.host}:{self.args.port}'
        for _ in range(C.web_ready_attempts):
            if self.web.poll() is not None:
                raise RuntimeError(f'web exited; inspect {name}.log')
            try:
                with urlopen(base + '/health', timeout=C.health_timeout_seconds):
                    return base
            except OSError:
                time.sleep(C.health_poll_seconds)
        raise RuntimeError('web startup timeout')

    def content_check(self, base, trace_id, phase):
        subprocess.run(['node', str(Path(__file__).with_name('content.mjs')), base, str(trace_id),
                        str(self.work), phase, self.args.playwright, self.args.chromium],
                       check=True, timeout=C.content_timeout_seconds)

    def run(self):
        report = {"status": "failed", "work": str(self.work)}
        try:
            self.runtime.prepare()
            task = self.work / "task"
            task.mkdir()
            prompt = (
                f"Work only in {task}. Perform these steps using tools, in separate tool calls: "
                f"1. List this directory. 2. Create numbers.txt containing numbers 1 through {C.task_number_count}, "
                "one per line. 3. Read numbers.txt. 4. Run a shell command to sum its numbers. "
                "5. Create result.txt containing the sum. 6. Read result.txt. "
                "This task includes the UTF-8 storage marker 火焰图. "
                "Finally reply exactly FLAMEGRAPH_E2E_OK. Do not skip tools.")
            launched = self.runtime.run(
                self.runtime.control_command("launch", "--", self.args.agent.resolve(),
                                             "--cli", "run", "--max-turns", str(C.agent_max_turns),
                                             "--prompt", prompt),
                timeout_seconds=self.args.agent_timeout, cwd=task)
            (self.work / "agent.log").write_text(launched.output)
            if launched.returncode or "FLAMEGRAPH_E2E_OK" not in launched.output:
                raise RuntimeError("real agent failed; inspect agent.log")
            ids = re.findall(r"trace trace-(\d+) entered Active", launched.output)
            if len(ids) != 1:
                raise RuntimeError(f"expected one captured trace, got {ids}")
            trace_id = int(ids[0])
            long_capture = LongContentCapture(self.runtime, self.args, self.work)
            long_trace_id = long_capture.run(task)
            for _ in range(C.extra_process_traces):
                extra = self.runtime.run(self.runtime.control_command("launch", "--", "/usr/bin/true"),
                                         timeout_seconds=C.process_timeout_seconds)
                if extra.returncode:
                    raise RuntimeError("additional real process capture failed")
            stopped = self.runtime.stop()
            if stopped is None or stopped.returncode:
                raise RuntimeError("daemon did not drain successfully")
            if (task / "result.txt").read_text().strip() != str(sum(range(1, C.task_number_count + 1))):
                raise RuntimeError("real tool task did not produce expected result")
            report["trace_id"] = trace_id
            report["long_trace_id"] = long_trace_id
            database = self.work / "data/actrail.sqlite"
            with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
                expected = [row[0] for row in connection.execute(
                    "SELECT ids.action_id FROM semantic_actions a "
                    "JOIN semantic_action_ids ids ON ids.action_key=a.action_key "
                    "WHERE a.trace_id=? AND a.action_valid_code=1", (trace_id,))]
                terminal_ends = [str(row[0]) for row in connection.execute(
                    "SELECT state.end_time FROM semantic_actions a "
                    "JOIN semantic_action_state state ON state.action_key=a.action_key "
                    "WHERE a.trace_id=? AND a.action_valid_code=1 "
                    "AND state.end_time IS NOT NULL ORDER BY a.start_time,a.action_key LIMIT 1", (trace_id,))]
                trace_ids = [row[0] for row in connection.execute(
                    "SELECT trace_id FROM traces ORDER BY created_at DESC, trace_id DESC")]
                content = CapturedRequest(connection, trace_id).read()
                long_capture.expected(connection, long_trace_id)
            (self.work / "content-expected.json").write_text(json.dumps(content, ensure_ascii=False))
            (self.work / "expected.json").write_text(json.dumps({"ids": expected, "terminal_ends": terminal_ends,
                                                               "trace_ids": trace_ids}))
            base = self.start_web('web')
            subprocess.run([
                'node', str(Path(__file__).with_name('browser.mjs')), base,
                str(trace_id), str(self.work), self.args.endpoint,
                self.args.playwright, self.args.chromium], check=True, timeout=C.browser_timeout_seconds)
            self.content_check(base, trace_id, 'full')
            subprocess.run(['node', str(Path(__file__).parent / 'long_content/browser.mjs'), base,
                            str(long_trace_id), str(self.work), self.args.playwright,
                            self.args.chromium], check=True, timeout=C.content_timeout_seconds)
            empty_config = self.work / 'resources-empty.conf'
            PatchBuilder().apply(self.work / 'actraild.conf').with_values({
                'resource_profiling': {'storage': {'database': {'sqlite': {
                    'path': str(self.work / 'resources-empty.sqlite'),
                }}}},
            }).build().save(empty_config)
            base = self.start_web('web-resources-empty', config=empty_config)
            subprocess.run(['node', str(Path(__file__).parent / 'browser/resource-run.mjs'), base,
                            str(trace_id), str(self.work), self.args.playwright, self.args.chromium, 'empty'],
                           check=True, timeout=C.browser_timeout_seconds)
            base = self.start_web('web-small-budget', ACTRAIL_WEB_TIMELINE_DEFAULT_LIMIT=str(C.timeline_small_budget),
                                  ACTRAIL_WEB_TIMELINE_MAX_LIMIT=str(C.timeline_small_budget), ACTRAIL_WEB_TIMELINE_CANDIDATES=str(C.timeline_small_budget))
            subprocess.run(['node', str(Path(__file__).with_name('small-budget.mjs')),
                            base, str(trace_id), str(self.work)], check=True, timeout=C.process_timeout_seconds)
            base = self.start_web('web-content-preview', ACTRAIL_WEB_CONTENT_NODE_PREVIEW_BYTES=str(C.content_preview_bytes))
            self.content_check(base, trace_id, 'preview')
            base = self.start_web('web-content-budget', ACTRAIL_WEB_CONTENT_NODE_BLOCK_BYTES=str(C.content_block_budget_bytes))
            self.content_check(base, trace_id, 'blocked')
            report["status"] = "passed"
        except Exception as error:
            report["error"] = str(error)
            raise
        finally:
            if self.web:
                self.web.terminate()
                try:
                    self.web.wait(timeout=C.web_stop_timeout_seconds)
                except subprocess.TimeoutExpired:
                    self.web.kill()
                    self.web.wait()
            self.runtime.stop()
            (self.work / "result.json").write_text(json.dumps(report, indent=C.json_indent) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, required=True, help="new isolated directory")
    parser.add_argument("--bin-dir", type=Path, default=REPO / C.bin_dir)
    parser.add_argument("--agent", type=Path, default=Path(C.agent_path))
    parser.add_argument("--agent-timeout", type=int, default=C.agent_timeout_seconds)
    parser.add_argument("--port", type=int, default=C.port)
    parser.add_argument("--endpoint", default=C.endpoint)
    parser.add_argument("--playwright", default=C.playwright_path)
    parser.add_argument("--chromium", default=C.chromium_path)
    Acceptance(parser.parse_args()).run()
