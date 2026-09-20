#!/usr/bin/env python3
"""Validate config modes using real CLI commands and a real xiaoO tool task."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from tests.v2.common.testing_env import AgentBinaryDiscovery


class ConfigInitAcceptance:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.bins = args.bin_dir.resolve()
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=False)
        self.report: dict = {"status": "running", "modes": {}}
        self.sequence = 0
        discovery = AgentBinaryDiscovery(ROOT)
        self.agent = args.xiaoo or discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
        if self.agent is None or not discovery.is_executable(self.agent):
            raise RuntimeError("real xiaoO unavailable; set --xiaoo or XIAOO_E2E_BINARY")
        self.environment = discovery.environment(self.agent)
        for name in ("actraild", "actrailctl", "actrailviewer"):
            if not os.access(self.bins / name, os.X_OK):
                raise RuntimeError(f"missing current release executable: {self.bins / name}")

    def run_command(self, command, *, cwd=None, timeout=30, success=True):
        self.sequence += 1
        path = self.out / f"command-{self.sequence:02d}.log"
        with path.open("w") as output:
            process = subprocess.Popen(
                [str(part) for part in command], cwd=cwd or ROOT,
                env=self.environment, stdout=output, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                result = process.wait(timeout=timeout)
            except BaseException:
                self.terminate(process)
                raise
        text = path.read_text(errors="replace")
        if success and result != 0:
            raise RuntimeError(f"command failed ({result}), see {path}:\n{text[-3000:]}")
        if not success and result == 0:
            raise AssertionError(f"expected command rejection, see {path}")
        return text

    @staticmethod
    def terminate(process: subprocess.Popen):
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=10)

    def init(self, path: Path, *arguments, success=True):
        return self.run_command(
            [self.bins / "actrailctl", "init", "--output", path, *arguments],
            success=success,
        )

    @staticmethod
    def document(path: Path):
        return tomllib.loads(path.read_text())

    def validate_cli(self):
        expected = {}
        for mode in ("default", "c", "C", "complete", "p", "P", "profile"):
            path = self.out / f"{mode}.toml"
            self.init(path, *([] if mode == "default" else ["--mode", mode]))
            expected[mode] = self.document(path)
        if any(expected[mode] != expected["default"] for mode in ("c", "C", "complete")):
            raise AssertionError("complete aliases differ from the default configuration")
        if any(expected[mode] != expected["p"] for mode in ("P", "profile")):
            raise AssertionError("profile aliases differ")
        for key in ("control", "storage"):
            if expected["p"][key] != expected["default"][key]:
                raise AssertionError(f"mode unexpectedly changes {key} paths/settings")
        if not expected["p"]["payload"]["tls"]["direct_dynamic_discovery_enabled"]:
            raise AssertionError("profile mode disabled default dynamic discovery")
        path = self.out / "p.toml"
        before = path.read_bytes()
        self.init(path)
        self.init(path, "--mode", "c", success=False)
        if path.read_bytes() != before:
            raise AssertionError("existing config was changed without --force")
        patch = self.out / "override.toml"
        patch.write_text('[semantic_retention.l0_llm_call]\nusage = "summary"\n')
        self.init(path, "--patch", patch, success=False)
        if path.read_bytes() != before:
            raise AssertionError("rejected patch changed existing config")
        self.init(path, "--mode", "p", "--force", "--patch", patch)
        if self.document(path)["semantic_retention"]["l0_llm_call"]["usage"] != "summary":
            raise AssertionError("user patch did not override mode")
        self.init(path, "--mode", "c", "--force")
        if self.document(path) != expected["default"]:
            raise AssertionError("forced complete initialization did not refresh defaults")
        invalid = self.out / "invalid.toml"
        self.init(invalid, "--mode", "invalid", success=False)
        if invalid.exists():
            raise AssertionError("invalid mode created a config")
        self.report["cli"] = "passed"

    def write_paths(self, work: Path):
        fields = {
            "control": {"socket_path": "control.sock", "pid_file": "daemon.pid", "log_path": "daemon.log"},
            "storage.sqlite": {"path": "actrail.sqlite"},
            "sandbox_evidence": {"path": "sandbox-evidence.sqlite"},
            "sandbox_alerts": {"path": "sandbox-alerts.sqlite"},
            "export.snapshot": {"directory": "export"},
            "payload.tls": {"sync_event_socket_path": "tls-sync.sock"},
            "cluster.report": {"spool_dir": "cluster-spool", "state_path": "cluster-state.sqlite"},
            "cluster.center": {"root_dir": "cluster"},
        }
        patch = work / "paths.toml"
        patch.write_text("\n".join(
            f"[{section}]\n" + "\n".join(
                f"{key} = {json.dumps(str(work / relative))}" for key, relative in paths.items()
            ) + "\n" for section, paths in fields.items()
        ))
        return patch

    def run_mode(self, mode: str, work: Path):
        work.mkdir()
        config = work / "operator.toml"
        self.init(config, "--mode", mode, "--patch", self.write_paths(work))
        (self.out / f"runtime-{mode}.toml").write_bytes(config.read_bytes())
        daemon_command = [self.bins / "actraild", "--config", config]
        control_command = [self.bins / "actrailctl", "--config", config]
        marker = "CONFIG_MODE_" + secrets.token_hex(8)
        (work / "input.txt").write_text(marker + "\n")
        prompt = (
            "Use the bash tool exactly once to run `cat input.txt | tee output.txt`. "
            "Then reply with only the exact text read from the file. Do not use any other tools."
        )
        with (self.out / f"daemon-{mode}.log").open("w") as log:
            daemon = subprocess.Popen(
                [str(part) for part in [*daemon_command, "run"]], cwd=ROOT,
                env=self.environment, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 60
                while not (work / "control.sock").exists():
                    if daemon.poll() is not None:
                        raise RuntimeError(f"{mode}: daemon exited during startup")
                    if time.monotonic() >= deadline:
                        raise TimeoutError(f"{mode}: daemon startup exceeded 60 seconds")
                    time.sleep(0.1)
                self.run_command([*control_command, "doctor"])
                launch = self.run_command(
                    [*control_command, "launch", "--", self.agent, "--cli", "run",
                     "--tools", "bash", "--max-turns", "4", "--prompt", prompt],
                    cwd=work, timeout=self.args.launch_timeout,
                )
                if marker not in launch or (work / "output.txt").read_text().strip() != marker:
                    raise AssertionError(f"{mode}: real agent did not complete the file/tool task")
                traces = re.findall(r"trace trace-(\d+) entered Active", launch)
                if len(traces) != 1:
                    raise AssertionError(f"{mode}: expected one trace, got {traces}")
            finally:
                try:
                    if daemon.poll() is None:
                        daemon.terminate()
                        daemon.wait(timeout=240)
                finally:
                    if daemon.poll() is None:
                        self.terminate(daemon)
                if daemon.returncode != 0:
                    raise RuntimeError(f"{mode}: daemon exited with {daemon.returncode}")
        graph_text = self.run_command(
            [self.bins / "actrailviewer", "--config", config, "--output-format", "json",
             "actions", "--trace-id", traces[0]], timeout=60,
        )
        (self.out / f"actions-{mode}.json").write_text(graph_text)
        graph = json.loads(graph_text)
        result = self.validate_actions(mode, graph, marker)
        with sqlite3.connect(f"file:{work / 'actrail.sqlite'}?mode=ro", uri=True) as db:
            retained = {
                table: db.execute(f"SELECT COUNT(*) FROM {table} WHERE trace_id=?", (int(traces[0]),)).fetchone()[0]
                for table in ("llm_request_manifests", "llm_request_blocks", "payload_segments")
            }
            if mode == "c":
                result["tool_result_storage"] = self.validate_stored_tool_result(
                    db, int(traces[0]), graph, marker,
                )
        if mode == "p" and any(retained.values()):
            raise AssertionError(f"profile retained content rows: {retained}")
        if mode == "c" and not all(retained[table] for table in ("llm_request_manifests", "llm_request_blocks")):
            raise AssertionError(f"complete lacks canonical request content: {retained}")
        result["retained_rows"] = retained
        self.report["modes"][mode] = result
        print(f"{mode}: real xiaoO tool task and retained evidence passed", flush=True)

    @classmethod
    def validate_stored_tool_result(cls, db, trace_id: int, graph: dict, marker: str):
        call_ids = {
            action["attributes"].get("llm.tool_call.id") for action in graph["actions"]
            if action["kind"] == "llm.tool_call"
        }
        for manifest_id, skeleton in db.execute(
            "SELECT manifest_id, skeleton_json FROM llm_request_manifests WHERE trace_id=?",
            (trace_id,),
        ):
            blocks = {
                ordinal: json.loads(encoded) for ordinal, encoded in db.execute(
                    "SELECT r.ordinal, b.encoded_bytes FROM llm_request_block_refs r "
                    "JOIN llm_request_blocks b ON b.block_id=r.block_id WHERE r.manifest_id=?",
                    (manifest_id,),
                )
            }
            body = cls.hydrate(json.loads(skeleton), blocks)
            messages = body.get("messages", [])
            for message in messages:
                if (message.get("role") == "tool" and message.get("tool_call_id") in call_ids
                        and marker in json.dumps(message.get("content"))):
                    return {"call_id": message["tool_call_id"], "marker_retained": True,
                            "last_message_role": messages[-1].get("role")}
        raise AssertionError("complete lacks stored tool result matching the observed call and marker")

    @classmethod
    def hydrate(cls, value, blocks):
        if isinstance(value, list):
            return [cls.hydrate(item, blocks) for item in value]
        if isinstance(value, dict):
            if set(value) == {"$actrail_llm_block"}:
                return blocks[value["$actrail_llm_block"]]
            return {key: cls.hydrate(item, blocks) for key, item in value.items()}
        return value

    @staticmethod
    def validate_actions(mode: str, graph: dict, marker: str):
        by_kind: dict[str, list] = {}
        for action in graph["actions"]:
            by_kind.setdefault(action["kind"], []).append(action)
        if not by_kind.get("agent.identity"):
            raise AssertionError(f"{mode}: no agent identity")
        requests = by_kind.get("llm.request", [])
        responses = by_kind.get("llm.response", [])
        complete_requests = [a for a in requests if a["status"] == "success"]
        successful_responses = [a for a in responses if a["status"] == "success"]
        if len(complete_requests) < 2 or len(successful_responses) < 2:
            raise AssertionError(f"{mode}: missing model exchanges around the tool call")
        for action in complete_requests + responses:
            if (action["status"] not in ("success", "error") or action["completeness"] != "complete"
                    or not action["start_time_unix_nanos"] or not action["end_time_unix_nanos"]):
                raise AssertionError(f"{mode}: missing complete timed model evidence")
        if any(a["attributes"].get("llm.response.done") != "true" for a in responses):
            raise AssertionError(f"{mode}: missing protocol completion")
        if mode == "p":
            forbidden = {
                "llm.request.message_preview", "llm.request.trajectory_id", "llm.request.body_json",
                "llm.response.content_text", "llm.response.output_text", "llm.response.reasoning_text",
                "llm.response.tool_calls_json", "llm.response.prompt_tokens",
                "llm.response.completion_tokens", "llm.response.total_tokens",
            }
            if any(forbidden.intersection(a["attributes"]) for a in graph["actions"]):
                raise AssertionError("profile retained disabled semantic content")
            if by_kind.get("llm.tool_call") or by_kind.get("llm.tool_result"):
                raise AssertionError("profile retained disabled tool projections")
            if any(a["attributes"].get("llm.request.content_state") != "none" for a in requests):
                raise AssertionError("profile request content state is not none")
        else:
            if not by_kind.get("llm.tool_call"):
                raise AssertionError("complete did not retain the real tool call")
            if not any(marker in a["attributes"].get("llm.response.content_text", "")
                       or marker in a["attributes"].get("llm.response.output_text", "") for a in responses):
                raise AssertionError("complete response content lacks the file marker")
        return {"status": "passed", "requests": len(requests), "responses": len(responses),
                "tool_calls": len(by_kind.get("llm.tool_call", []))}

    def run(self):
        try:
            self.validate_cli()
            # Short isolated socket paths; evidence is copied to --out before cleanup.
            with tempfile.TemporaryDirectory(prefix="aci-") as temporary:
                for mode in ("p", "c"):
                    self.run_mode(mode, Path(temporary) / mode)
            self.report["status"] = "passed"
        except BaseException as error:
            self.report["status"] = "failed"
            self.report["error"] = str(error)
            raise
        finally:
            (self.out / "result.json").write_text(json.dumps(self.report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--out", type=Path, required=True, help="new evidence directory")
    parser.add_argument("--xiaoo", type=Path)
    parser.add_argument("--launch-timeout", type=int, default=180)
    ConfigInitAcceptance(parser.parse_args()).run()
