"""Verify tool-result retention through real OpenCode, HTTPS MaaS and the storage reader."""

from __future__ import annotations

import argparse
import json
import shutil
import sqlite3
import subprocess
import tomllib
from pathlib import Path
from types import SimpleNamespace

from scripts.bench.payload.benchmark import BenchmarkLock, HERE, ROOT, PayloadBenchmark


class ToolResultsAcceptance:
    def __init__(self, out: Path, agent: Path):
        self.out = out.resolve()
        self.agent = agent.resolve()
        self.bin_dir = ROOT / "target/release"

    def run(self) -> None:
        self.out.mkdir(parents=True, exist_ok=False)
        reports = []
        with BenchmarkLock(Path("/run/lock/actrail-v2-regression.lock"), 5):
            for name, mode, export, limit in (
                ("disabled", "P", "none", 131072),
                ("metadata", "C", "none", 131072),
                ("metadata_request_none", "C", "none", 131072),
                ("exported", "C", "canonical_json", 131072),
                ("too_large", "C", "canonical_json", 16),
            ):
                reports.append(self.scenario(name, mode, export, limit))
            self.reject_conflict()
        (self.out / "acceptance.json").write_text(json.dumps({
            "status": "passed", "scenarios": reports, "conflict_rejected": True,
        }, indent=2) + "\n")

    def scenario(self, name: str, mode: str, export: str, limit: int) -> dict:
        config = self.out / f"config-{name}"
        config.mkdir()
        shutil.copy2(HERE / "configs/benchmark.toml", config / "benchmark.toml")
        patch = (HERE / f"configs/{mode}.toml").read_text()
        if mode == "C":
            patch += (
                "\n[semantic_retention.l0_llm_call]\n"
                f'tool_result_content_export = "{export}"\n'
                f"tool_result_content_export_max_bytes = {limit}\n"
            )
            if name == "metadata_request_none":
                patch += ('request_content = "none"\nrequest_body_export = "none"\n'
                          '[semantic_retention.l0_llm_call.trajectory]\nenabled = false\n')
        (config / f"{mode}.toml").write_text(patch)
        args = SimpleNamespace(
            config_dir=config, out=self.out / name, bin_dir=self.bin_dir,
            agent_kind="opencode", agent_bin=self.agent, cc="cc", skip_build=True,
            keep_runtime=True, modes=[mode], workloads=["agent"], perf=False,
            warmups=0, rounds=1, agent_turns=4, agent_tpot_ms=[0], agent_input_bytes=65536,
        )
        benchmark = PayloadBenchmark(args)
        benchmark.run()
        sample = benchmark.report["samples"][0]
        trace_id = sample["collection"]["traces"][0][0]
        rendered = subprocess.run([
            str(self.bin_dir / "actrailviewer"), "--config",
            str(args.out / f"runtime-{mode}/actraild.conf"), "--output-format", "json",
            "actions", "--trace-id", str(trace_id),
        ], check=True, capture_output=True, text=True, timeout=30)
        (args.out / "actions.json").write_text(rendered.stdout)
        document = json.loads(rendered.stdout)
        actions = {action["action_id"]: action for action in document["actions"]}
        if name == "metadata_request_none":
            self.verify_request_none(args.out / f"runtime-{mode}", actions, trace_id)
        results = [a for a in actions.values() if a["kind"] == "llm.tool_result"]
        links = [link for link in document["links"] if link["role"] == "llm.tool_call.result"]
        expected = 0 if name == "disabled" else 3
        if len(results) != expected or len(links) != expected:
            raise RuntimeError(f"{name}: expected {expected} tool results and bindings")
        for result in results:
            attrs = result["attributes"]
            request = actions[attrs["llm.tool_result.request_action_id"]]
            if (request["kind"] != "llm.request"
                    or request["trace_id"] != result["trace_id"]
                    or request["process"] != result["process"]
                    or attrs["llm.tool_result.binding_state"] != "bound"):
                raise RuntimeError(f"{name}: tool result request identity mismatch")
            parents = [link for link in links if link["child_action_id"] == result["action_id"]]
            if len(parents) != 1 or not parents[0]["valid"] or parents[0]["origin"] != "observed":
                raise RuntimeError(f"{name}: missing observed tool binding")
            call = actions[parents[0]["parent_action_id"]]
            if (call["kind"] != "llm.tool_call"
                    or call["attributes"]["llm.tool_call.id"] != attrs["llm.tool_result.id"]):
                raise RuntimeError(f"{name}: tool call ID mismatch")
            response_id = call["attributes"]["llm.tool_call.response_action_id"]
            if actions[response_id]["kind"] != "llm.response" or not any(
                link["parent_action_id"] == response_id
                and link["child_action_id"] == call["action_id"]
                and link["role"] == "llm.response.tool_call" and link["valid"]
                for link in document["links"]
            ):
                raise RuntimeError(f"{name}: response/tool relationship missing")
            state = attrs["llm.tool_result.content_export_state"]
            content = attrs.get("llm.tool_result.content_json")
            length = attrs.get("llm.tool_result.content_bytes")
            if name in ("metadata", "metadata_request_none"):
                valid = state == "none" and content is None and length is None
            elif name == "exported":
                valid = (state == "exported" and content is not None and length is not None
                         and len(content.encode()) == int(length) <= limit)
                if valid:
                    json.loads(content)
            else:
                valid = state == "too_large" and content is None and length is not None and int(length) > limit
            if not valid:
                raise RuntimeError(f"{name}: unexpected export state or content size")
        return {"name": name, "results": len(results), "bindings": len(links),
                "workload": sample["workload_result"], "trace_id": trace_id}

    @staticmethod
    def verify_request_none(runtime: Path, actions: dict, trace_id: int) -> None:
        retention = tomllib.loads((runtime / "actraild.conf").read_text())["semantic_retention"]["l0_llm_call"]
        if (retention["request_content"] != "none" or retention["request_body_export"] != "none"
                or retention["trajectory"]["enabled"] or not retention["tool_results_enabled"]):
            raise RuntimeError("request-none/tool-results configuration did not resolve as requested")
        requests = [action for action in actions.values() if action["kind"] == "llm.request"]
        if len(requests) != 4 or any(action["attributes"].get("llm.request.content_state") != "none"
                                     or "llm.request.message_preview" in action["attributes"] for action in requests):
            raise RuntimeError("request-none retained request content")
        with sqlite3.connect(f"file:{runtime / 'data/actrail.sqlite'}?mode=ro", uri=True) as database:
            if database.execute("SELECT COUNT(*) FROM llm_request_manifests WHERE trace_id=?", (trace_id,)).fetchone()[0]:
                raise RuntimeError("request-none retained request manifests")

    def reject_conflict(self) -> None:
        patch = self.out / "conflict.toml"
        patch.write_text('[semantic_retention.l0_llm_call]\ntool_results_enabled = false\n'
                         'tool_result_content_export = "canonical_json"\n')
        result = subprocess.run([
            str(self.bin_dir / "actrailctl"), "init", "--output", str(self.out / "conflict.conf"),
            "--patch", str(patch),
        ], capture_output=True, text=True, timeout=30)
        (self.out / "conflict.log").write_text(result.stdout + result.stderr)
        if result.returncode == 0 or "tool_results_enabled = true" not in result.stdout + result.stderr:
            raise RuntimeError("conflicting tool-result configuration was not explicitly rejected")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--agent-bin", type=Path, default=Path(shutil.which("opencode") or "opencode"))
    args = parser.parse_args()
    ToolResultsAcceptance(args.out, args.agent_bin).run()
