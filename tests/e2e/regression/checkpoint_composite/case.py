"""Save and restore files and real Xiaoo context through the daemon coordinator."""

from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import subprocess
from pathlib import Path

from tests.e2e.common.core import TestRunDirectories, TestResult, TestStatus
from tests.e2e.common.patch_builder import PatchBuilder
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.regression.checkpoint_btrfs.case import CheckpointBtrfsCase
from tests.e2e.regression.checkpoint_btrfs.constants import CASE_CONSTANTS as BTRFS

from .xiaoo import XiaooSession


class CheckpointCompositeCase(CheckpointBtrfsCase):
    """Use the Btrfs fixture's filesystem, control commands and cleanup."""
    case_id = 'checkpoint_composite'
    description = 'Save and restore real Xiaoo context and Btrfs files together'
    default_enabled = False



    def __init__(self, directories: TestRunDirectories):
        super().__init__(directories)
        self._xiaoo = XiaooSession(self._work / "xiaoo")
        target = self._library.parent / "libactrail_ckpt_cxt_xiaoo.so"
        self._context_library = Path(os.environ.get("ACTRAIL_CHECKPOINT_XIAOO_LIBRARY", target))
        self._trace_id = ""
        self._saved_checkpoint = ""
        self._alpha = "ALPHA_" + self._xiaoo.name.replace("-", "_")
        self._beta = "BETA_" + self._xiaoo.name.replace("-", "_")
        self._other_xiaoo = XiaooSession(self._work / "other-xiaoo")
        self._other_project = self._mnt / "workspaces/other-project"
        self._other_file = self._other_project / "a.txt"
        self._other_trace = ""
        self._other_saved = ""
        self._gamma = "GAMMA_" + self._other_xiaoo.name.replace("-", "_")
        self._delta = "DELTA_" + self._other_xiaoo.name.replace("-", "_")

    def _setup(self) -> str:
        # Both backends share this test's daemon; each owns its native snapshot data.
        work = self._work
        for directory in ("run", "log", "data", "mnt"):
            (work / directory).mkdir(parents=True, exist_ok=True)
        image = work / "filesystem.img"
        with image.open("wb") as handle:
            handle.truncate(BTRFS.filesystem_bytes)
        self._run("mkfs.btrfs", "-q", image)
        self._run("mount", "-o", "loop", image, self._mnt)
        self._mounted = True
        (self._mnt / "workspaces").mkdir()
        (self._mnt / ".actrail").mkdir()
        self._run("btrfs", "subvolume", "create", self._project)
        self._file.write_text("before\n", encoding="utf-8")
        self._xiaoo.prepare()
        (work / "btrfs.json").write_text(json.dumps({
            "btrfs_path": shutil.which("btrfs"), "findmnt_path": shutil.which("findmnt"),
            "copy_path": shutil.which("cp"), "snapshot_root": str(self._mnt / ".actrail"),
            "allowed_workspace_roots": [str(self._mnt / "workspaces")],
        }), encoding="utf-8")
        (work / "xiaoo-runtime.json").write_text("{}", encoding="utf-8")
        (work / "alert-forwarding.json").write_text(json.dumps({
            "enabled": False, "all_categories": False, "categories": [],
        }), encoding="utf-8")
        patch = PatchBuilder().with_work_dir(work).with_values({
            "supervision": {"startup_wait_ms": BTRFS.daemon_startup_wait_ms},
            "alert_forwarding": {"socket_path": str(work / "run/alert-proxy.sock"),
                                 "plugin_config_path": str(work / "alert-forwarding.json")},
            "sandbox_evidence": {"path": str(work / "data/sandbox-evidence.sqlite")},
            "sandbox_alerts": {"path": str(work / "data/sandbox-alerts.sqlite")},
            "payload": {"tls": {"capture_backend": "tls-sync"}},
            "checkpoint": {"enabled": True, "catalog_path": str(work / "data/checkpoint.db"),
                           "backends": [
                               {"name": BTRFS.backend_instance, "library": str(self._library),
                                "config": str(work / "btrfs.json")},
                               {"name": "xiaoo-context", "library": str(self._context_library),
                                "config": str(work / "xiaoo-runtime.json")},
                           ]},
        }).build().save(work / "checkpoint.patch.toml")
        self._ctl("init", "--force", "--patch", patch)
        self._scopes.write_text(self._scope("project", self._project) +
                                self._context_config(work / "xiaoo.json"), encoding="utf-8")
        self._run(self._binary("actraild"), "--config", self._config, "start",
                  timeout=BTRFS.launch_timeout_seconds)
        self._daemon_started = True
        return "fresh daemon and Xiaoo defaults; isolated Btrfs and both backend libraries"

    @staticmethod
    def _context_config(path: Path) -> str:
        return ('[[backends]]\nname = "xiaoo-context"\n'
                f'config = {json.dumps(str(path))}\n')

    def _launch_other(self) -> str:
        # Create the second task's private configs after the daemon has loaded its libraries.
        # These owned fixture files prove new Trace instances use the runtime loaded at startup.
        for filename in ("btrfs.json", "xiaoo-runtime.json"):
            (self._work / filename).write_text("runtime file changed after daemon startup", encoding="utf-8")
        self._run("btrfs", "subvolume", "create", self._other_project)
        self._other_file.write_text(self._gamma + "\n", encoding="utf-8")
        self._other_xiaoo.prepare()
        context_config = self._work / "other-xiaoo.json"
        context_config.write_text(json.dumps({"endpoint": str(self._other_xiaoo.endpoint)}),
                                  encoding="utf-8")
        launch_config = self._work / "other-checkpoint.toml"
        launch_config.write_text(
            self._scope("project", self._other_project) + self._context_config(context_config),
            encoding="utf-8")
        launch = [str(self._binary("actrailctl")), "--config", str(self._config), "launch",
                  "--name", self._other_xiaoo.name, "--checkpoint", str(launch_config), "--"]
        self._other_xiaoo.start(launch, self._other_project)
        self._other_trace = self._wait("the second Xiaoo Trace",
                                       lambda: self._trace(self._other_xiaoo.name))
        if self._other_trace == self._trace_id:
            raise AssertionError("two Xiaoo launches share one Trace")
        self._other_xiaoo.prompt(f"记住本会话唯一口令为 {self._gamma}。只回复这个口令，不调用工具。")
        operation = self._operation(self._ctl("checkpoint", "save", self._other_trace))
        self._succeeded(operation)
        view = self._ctl("checkpoint", "operation", operation)
        if not (created := re.search(r"created=(checkpoint-\d+)", view)):
            raise AssertionError(f"second Trace save did not publish a checkpoint: {view}")
        self._other_saved = created.group(1)
        self._other_file.write_text(self._delta + "\n", encoding="utf-8")
        self._other_xiaoo.prompt(f"将本会话唯一口令替换为 {self._delta}。只回复这个新口令，不调用工具。")
        self._assert_other(self._delta)
        return ("second real Trace used the daemon's loaded runtime and a new private config "
                "after both runtime files changed, without restarting the daemon")

    def _reject_runtime_override(self) -> str:
        # Launch reaches the real backend factory while both existing Agents remain alive.
        trace_config = self._work / "invalid-xiaoo-trace.json"
        trace_config.write_text(json.dumps({"endpoint": str(self._xiaoo.endpoint),
                                             "request_timeout_ms": 1}), encoding="utf-8")
        launch_config = self._work / "invalid-checkpoint.toml"
        launch_config.write_text(self._scope("project", self._project) +
                                 self._context_config(trace_config), encoding="utf-8")
        ran = self._work / "invalid-launch-ran"
        result = subprocess.run(self._launch(self._xiaoo.name + "-invalid",
            f"touch {shlex.quote(str(ran))}", launch_config), cwd=self._directories.repo,
            capture_output=True, text=True, timeout=BTRFS.launch_timeout_seconds)
        if result.returncode == 0 or ran.exists():
            raise AssertionError("a Trace changed runtime settings and executed its process")
        if "request_timeout_ms" not in result.stderr or "unknown field" not in result.stderr:
            raise AssertionError(f"launch failed for an unexpected reason: {result.stderr}")
        (self._work / "rejected-runtime-override.txt").write_text(result.stderr, encoding="utf-8")
        self._xiaoo.prompt("现在本会话唯一口令是什么？只回复口令，不调用工具。")
        assistant = [message for message in self._xiaoo.messages()
                     if message.get("role") == "assistant"]
        if not assistant or self._beta not in json.dumps(assistant[-1], ensure_ascii=False):
            raise AssertionError("existing Agent lost its context after the rejected launch")
        self._assert_other(self._delta)
        self._succeeded(self._operation(self._ctl("checkpoint", "save", self._other_trace)))
        return "backend rejected a Trace runtime override before exec; both real Agents continue"

    def _assert_other(self, expected: str) -> None:
        if self._other_file.read_text(encoding="utf-8").strip() != expected:
            raise AssertionError("second Trace filesystem was changed by another Trace")
        assistant = [message for message in self._other_xiaoo.messages()
                     if message.get("role") == "assistant"]
        if not assistant or expected not in json.dumps(assistant[-1], ensure_ascii=False):
            raise AssertionError("second Trace context does not match its own saved state")

    def _launch_xiaoo(self) -> str:
        (self._work / "xiaoo.json").write_text(
            json.dumps({"endpoint": str(self._xiaoo.endpoint)}), encoding="utf-8")
        launch = [str(self._binary("actrailctl")), "--config", str(self._config), "launch",
                  "--name", self._xiaoo.name, "--checkpoint", str(self._scopes), "--"]
        self._xiaoo.start(launch, self._project)
        self._trace_id = self._wait("the Xiaoo Trace", lambda: self._trace(self._xiaoo.name))
        if self._checkpoint_ids():
            raise AssertionError("a composite checkpoint was published before context initialization")
        return "real Xiaoo launched and created its socket without an initial context snapshot"

    def _files_only(self) -> str:
        # A loaded context backend needs no instance configuration when this launch omits it.
        if (self._work / "xiaoo.json").exists():
            raise AssertionError("context config must not exist during files-only selection")
        config = self._work / "files-only.toml"
        config.write_text(self._scope("project", self._project), encoding="utf-8")
        finished = self._work / "files-only-finish"
        name = self._xiaoo.name + "-files-only"
        process = self._start(self._launch(name,
            f"while [ ! -e {shlex.quote(str(finished))} ]; do sleep 0.1; done", config))
        trace = self._wait("files-only Trace", lambda: self._trace(name))
        self._wait("files-only initial checkpoint", lambda: self._listed(
            trace, r"^checkpoint-0 .*restorable"))
        finished.touch()
        self._wait_agent(process)
        self._wait("files-only automatic release", lambda: self._listed(
            trace, r"^checkpoint-0 .*release=released"))
        self._wait("files-only Btrfs cleanup",
                   lambda: not list((self._mnt / ".actrail/snapshots").glob("*/*/*")))
        return "files-only launch saved and released with no context JSON despite its loaded backend"

    def _checkpoint_ids(self) -> list[str]:
        return re.findall(r"^checkpoint-\d+", self._ctl("checkpoint", "list", self._trace_id), re.M)

    def _wait(self, what, ready, timeout=None):
        return super()._wait(what, ready, self._xiaoo.timeout if timeout is None else timeout)

    def _initialize_context(self) -> str:
        # The first genuine LLM request activates context capture through TLS-sync.
        self._xiaoo.prompt(f"记住本会话唯一口令为 {self._alpha}。只回复这个口令，不调用工具。")
        return "first real LLM turn completed; the following save verifies context activation"

    def _save_alpha(self) -> str:
        self._file.write_text(self._alpha + "\n", encoding="utf-8")
        operation = self._operation(self._ctl("checkpoint", "save", self._trace_id))
        self._succeeded(operation)
        view = self._ctl("checkpoint", "operation", operation)
        if not (created := re.search(r"created=(checkpoint-\d+)", view)):
            raise AssertionError(f"manual save did not publish a checkpoint: {view}")
        self._saved_checkpoint = created.group(1)
        if not list((self._mnt / ".actrail/snapshots").glob("*/*/*")):
            raise AssertionError("the composite save has no matching real Btrfs snapshot")
        return f"{self._saved_checkpoint} saved ALPHA file and initialized Xiaoo context"

    def _change_to_beta(self) -> str:
        self._file.write_text(self._beta + "\n", encoding="utf-8")
        self._xiaoo.prompt(f"将本会话唯一口令替换为 {self._beta}。只回复这个新口令，不调用工具。")
        messages = self._xiaoo.messages()
        transcript = json.dumps(messages, ensure_ascii=False)
        if self._beta not in transcript:
            raise AssertionError("real Xiaoo did not record the BETA context before restoration")
        self._expect(self._beta, "before restore")
        # A later real request must take an automatic composite snapshot.
        operations = self._ctl("checkpoint", "operations", self._trace_id)
        if not any("trigger=llm_boundary" in line and "outcome=succeeded" in line
                   for line in operations.splitlines()):
            raise AssertionError(f"no successful automatic LLM-boundary save: {operations}")
        return "file and exported conversation contain BETA; automatic boundary save succeeded"

    def _restore_alpha(self) -> str:
        operation = self._restore_waiting(self._trace_id, self._saved_checkpoint)
        # This real request releases the waiting restore through the normal boundary path.
        self._xiaoo.send("请只回复 READY，不调用工具。")
        self._succeeded(operation)
        self._xiaoo.finish_turn()
        self._expect(self._alpha, "after composite restore")
        self._xiaoo.prompt("现在本会话唯一口令是什么？只回复口令，不调用工具。")
        messages = self._xiaoo.messages()
        transcript = json.dumps(messages, ensure_ascii=False)
        assistant = [message for message in messages if message.get("role") == "assistant"]
        if self._alpha not in transcript or self._beta in transcript:
            raise AssertionError("restored conversation does not match the saved ALPHA context")
        if not assistant or self._alpha not in json.dumps(assistant[-1], ensure_ascii=False):
            raise AssertionError("the real agent did not answer using its restored ALPHA context")
        (self._work / "restored-conversation.json").write_text(
            json.dumps(messages, ensure_ascii=False, indent=2), encoding="utf-8")
        self._assert_other(self._delta)
        return "ALPHA file and context restored; the other Trace retains its DELTA file and context"

    def _delete_and_disconnect(self) -> str:
        self._ctl("checkpoint", "delete", self._trace_id, self._saved_checkpoint)
        self._wait("target snapshot release", lambda: self._listed(
            self._trace_id, rf"^{re.escape(self._saved_checkpoint)} .*release=released"))
        self._ctl("checkpoint", "disconnect", self._trace_id)
        self._wait("first Trace Btrfs snapshots to be released",
                   lambda: not list((self._mnt / ".actrail/snapshots").glob(
                       f"{self._trace_id}-*/*/*")))
        self._assert_other(self._delta)
        try:
            self._ctl("checkpoint", "save", self._trace_id)
        except AssertionError as error:
            if "disconnect" not in str(error):
                raise
        else:
            raise AssertionError("disconnected Trace still accepts a save")
        self._xiaoo.prompt("请只回复 CONTINUED，不调用工具。")
        assistant = [message for message in self._xiaoo.messages()
                     if message.get("role") == "assistant"]
        if not assistant or "CONTINUED" not in json.dumps(assistant[-1], ensure_ascii=False):
            raise AssertionError("Xiaoo did not continue after disconnect")
        return "target deleted, both backends disconnected, files released and real Agent continued"

    def _restore_other(self) -> str:
        operation = self._restore_waiting(self._other_trace, self._other_saved)
        self._other_xiaoo.send("请只回复 READY，不调用工具。")
        self._succeeded(operation)
        self._other_xiaoo.finish_turn()
        self._other_xiaoo.prompt("现在本会话唯一口令是什么？只回复口令，不调用工具。")
        self._assert_other(self._gamma)
        transcript = json.dumps(self._other_xiaoo.messages(), ensure_ascii=False)
        if self._delta in transcript or self._alpha in transcript or self._beta in transcript:
            raise AssertionError("second Trace contains context outside its saved checkpoint")
        self._expect(self._alpha, "after the other Trace restored")
        self._succeeded(self._operation(self._ctl("checkpoint", "save", self._other_trace)))
        return "second Trace restored its GAMMA state and saved again after the first disconnected"

    def _end_other(self) -> str:
        self._other_xiaoo.tmux("send-keys", "-t", self._other_xiaoo.name, "C-c")
        self._wait("second Xiaoo to exit", lambda: self._other_xiaoo.tmux(
            "display-message", "-p", "-t", self._other_xiaoo.name, "#{pane_dead}").strip() == "1")
        self._wait("second Trace automatic cleanup result", lambda: self._listed(
            self._other_trace, rf"^{re.escape(self._other_saved)} .*release=(released|failed)"))
        self._wait("all Btrfs snapshots to be released",
                   lambda: not list((self._mnt / ".actrail/snapshots").glob("*/*/*")))
        self._refused_restore(self._other_trace)
        released = self._ctl("checkpoint", "list", self._other_trace)
        (self._work / "ended-trace-checkpoints.txt").write_text(released, encoding="utf-8")
        self._xiaoo.prompt("请只回复 STILL_RUNNING，不调用工具。")
        assistant = [message for message in self._xiaoo.messages()
                     if message.get("role") == "assistant"]
        if not assistant or "STILL_RUNNING" not in json.dumps(assistant[-1], ensure_ascii=False):
            raise AssertionError("the other Agent could not continue after Trace exit cleanup")
        if "release=failed" in released:
            if "xiaoo-context" not in released or "release_error=" not in released:
                raise AssertionError(f"unexpected automatic cleanup failure: {released}")
            return ("real Xiaoo exit automatically released all Btrfs snapshots; the closed "
                    "Xiaoo socket caused a recorded context release failure local to this Trace; "
                    "restores are refused and the other real Agent continues")
        return "real Xiaoo exit automatically released its snapshots; the other real Agent continues"

    def _teardown(self) -> str | None:
        # Release any failure pause before closing the TUI, then unmount the owned filesystem.
        for trace in (self._trace_id, self._other_trace):
            if trace and self._daemon_started:
                try:
                    self._ctl("checkpoint", "resume", trace)
                except Exception:
                    pass
        try:
            self._xiaoo.close()
            self._other_xiaoo.close()
        finally:
            failure = super()._teardown()
        if not self._mnt.is_mount():
            (self._work / "filesystem.img").unlink(missing_ok=True)
        return failure

    def run(self, context: TestingContextSingleton) -> TestResult:
        for library in (self._library, self._context_library):
            if not library.is_file():
                return TestResult(TestStatus.FAILED, f"backend library not found: {library}")
        if missing := self._missing() or self._xiaoo.missing():
            return TestResult(TestStatus.SKIPPED, missing)
        results: dict[str, TestResult] = {}
        steps = [("setup", self._setup), ("files_only", self._files_only),
                 ("launch", self._launch_xiaoo),
                 ("context_boundary", self._initialize_context), ("save_alpha", self._save_alpha),
                 ("change_beta", self._change_to_beta), ("launch_other", self._launch_other),
                 ("reject_runtime_override", self._reject_runtime_override),
                 ("restore_alpha", self._restore_alpha),
                 ("delete_disconnect", self._delete_and_disconnect),
                 ("restore_other", self._restore_other), ("trace_end", self._end_other)]
        try:
            for name, step in steps:
                context.report_progress({
                    'setup': 'Prepare the composite checkpoint environment',
                    'files_only': 'Check file checkpoints before context capture',
                    'launch': 'Start XiaoO',
                    'context_boundary': 'Initialize Agent context capture',
                    'save_alpha': 'Save the initial file and Agent context',
                    'change_beta': 'Change the file and Agent context',
                    'launch_other': 'Start a second trace',
                    'reject_runtime_override': 'Check rejection of runtime overrides',
                    'restore_alpha': 'Restore the initial file and Agent context',
                    'delete_disconnect': 'Check checkpoint deletion and disconnection',
                    'restore_other': 'Restore the second trace',
                    'trace_end': 'Check checkpoints after the second trace ends',
                }[name])
                try:
                    results[name] = TestResult(TestStatus.PASSED, step())
                except Exception as error:
                    results[name] = TestResult(TestStatus.FAILED, f"{error}\n{self._daemon_log()}")
                    break
        finally:
            if failure := self._teardown():
                results["cleanup"] = TestResult(TestStatus.FAILED, failure)
        return TestResult(TestStatus.COMPOSITE, "real Xiaoo and Btrfs composite checkpoint", results)
