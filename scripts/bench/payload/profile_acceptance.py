"""Check retained evidence from a completed real-agent P/C benchmark."""

from __future__ import annotations

import argparse
import json
import sqlite3
import subprocess
from pathlib import Path

from scripts.bench.payload.benchmark import ROOT


class ProfileAcceptance:
    def __init__(self, directory: Path, bin_dir: Path = ROOT / "target/release"):
        self.directory = directory.resolve()
        self.bin_dir = bin_dir.resolve()

    def expected_completeness(self, action: dict) -> str:
        return "complete"

    def expected_request_content_state(self, action: dict) -> str:
        return "none"

    def run(self) -> None:
        report = json.loads((self.directory / "results.json").read_text())
        if report["status"] != "passed":
            raise RuntimeError("profile acceptance requires a completed successful real-agent run")
        evidence = []
        for sample in report["samples"]:
            if sample["mode"] not in ("P", "C") or not sample["workload"].startswith("agent-"):
                continue
            evidence.append(self.trace(sample, report["settings"]["agent_turns"]))
        if not evidence:
            raise RuntimeError("no observed real-agent traces to verify")
        (self.directory / "profile-acceptance.json").write_text(json.dumps({
            "status": "passed", "traces": evidence,
        }, indent=2) + "\n")

    def trace(self, sample: dict, turns: int) -> dict:
        mode = sample["mode"]
        trace_id = sample["collection"]["traces"][0][0]
        runtime = self.directory / f"runtime-{mode}"
        output = subprocess.run([
            str(self.bin_dir / "actrailviewer"), "--config", str(runtime / "actraild.conf"),
            "--output-format", "json", "actions", "--trace-id", str(trace_id),
        ], check=True, text=True, capture_output=True, timeout=30)
        (self.directory / f"actions-{mode}-{trace_id}.json").write_text(output.stdout)
        graph = json.loads(output.stdout)
        by_kind = {}
        for action in graph["actions"]:
            by_kind.setdefault(action["kind"], []).append(action)
        by_id = {action["action_id"]: action for action in graph["actions"]}
        for kind in ("llm.request", "llm.response", "llm.call"):
            if len(by_kind.get(kind, [])) != turns:
                raise RuntimeError(f"{mode}/{trace_id}: missing {kind}")
            for action in by_kind[kind]:
                if not action["start_time_unix_nanos"]:
                    raise RuntimeError(f"{mode}/{trace_id}: missing LLM start timestamp")
                # llm.call is an inferred grouping in the existing storage model;
                # request/response actions carry the protocol completion evidence.
                if kind != "llm.call" and (action["status"] != "success"
                        or action["completeness"] != self.expected_completeness(action) or not action["end_time_unix_nanos"]):
                    raise RuntimeError(f"{mode}/{trace_id}: missing complete timed LLM evidence")
        for role, kind in (("llm.call.request", "llm.request"), ("llm.call.response", "llm.response")):
            links = [link for link in graph["links"] if link["role"] == role and link["valid"]]
            if len(links) != turns or len({link["child_action_id"] for link in links}) != turns:
                raise RuntimeError(f"{mode}/{trace_id}: invalid LLM relationship coverage")
            for link in links:
                parent, child = by_id[link["parent_action_id"]], by_id[link["child_action_id"]]
                if (parent["kind"] != "llm.call" or child["kind"] != kind
                        or parent["process"] != child["process"]):
                    raise RuntimeError(f"{mode}/{trace_id}: cross-process LLM relationship")
        for response in by_kind["llm.response"]:
            if response["attributes"].get("llm.response.done") != "true":
                raise RuntimeError(f"{mode}/{trace_id}: missing protocol completion")
        with sqlite3.connect(f"file:{runtime / 'data/actrail.sqlite'}?mode=ro", uri=True) as db:
            manifests = db.execute("SELECT COUNT(*) FROM llm_request_manifests WHERE trace_id=?", (trace_id,)).fetchone()[0]
            blocks = db.execute("SELECT COUNT(*) FROM llm_request_blocks WHERE trace_id=?", (trace_id,)).fetchone()[0]
            payload = db.execute("SELECT COALESCE(SUM(length(bytes)),0) FROM payload_segments WHERE trace_id=?", (trace_id,)).fetchone()[0]
        if mode == "P":
            if manifests or blocks or payload or by_kind.get("llm.tool_call") or by_kind.get("llm.tool_result"):
                raise RuntimeError(f"{mode}/{trace_id}: disabled content or tools were retained")
            forbidden = {
                "llm.request.message_preview", "llm.request.trajectory_id", "llm.request.body_json",
                "llm.response.content_text", "llm.response.reasoning_text", "llm.response.tool_calls_json",
                "llm.response.prompt_tokens", "llm.response.completion_tokens", "llm.response.total_tokens",
            }
            for action in graph["actions"]:
                if forbidden.intersection(action["attributes"]):
                    raise RuntimeError(f"{mode}/{trace_id}: disabled semantic attributes retained")
            if any(a["attributes"].get("llm.request.content_state") != self.expected_request_content_state(a) for a in by_kind["llm.request"]):
                raise RuntimeError(f"{mode}/{trace_id}: request retention state mismatch")
        elif manifests != turns or not blocks or len(by_kind.get("llm.tool_call", [])) != turns - 1:
            raise RuntimeError(f"{mode}/{trace_id}: full content coverage missing")
        return {"mode": mode, "trace_id": trace_id, "llm_pairs": turns,
                "manifests": manifests, "blocks": blocks, "payload_bytes": payload}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    args = parser.parse_args()
    ProfileAcceptance(args.directory, args.bin_dir).run()
