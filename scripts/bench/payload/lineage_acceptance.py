"""Validate real benchmark command relationships and offline OTLP readback.

This reads completed agent traces. It does not manufacture late/conflicting
events or exercise the web UI. Command-tree checks use persisted action links.
"""
from __future__ import annotations

import argparse
import collections
import json
import sqlite3
import subprocess
from pathlib import Path


class LineageAcceptance:
    FILE_KINDS = {"file.read", "file.write", "file.modify", "file.tty_io",
                  "file.bulk_read", "fs.enumerate"}
    COMMAND_ROLES = {
        "command.contains_file_access": FILE_KINDS,
        "command.contains_process_fork_attempt": {"process.fork_attempt"},
        "command.contains_command_invocation": {"command.invocation", "agent.invocation"},
        "command.contains_llm_call": {"llm.call"},
    }
    AGENT_ROLE = "agent.performed_action"

    def __init__(self, directory: Path, bin_dir: Path):
        self.directory = directory.resolve()
        self.viewer = bin_dir.resolve() / "actrailviewer"
        self.output = self.directory / "lineage-acceptance"

    @staticmethod
    def require(condition, message):
        if not condition:
            raise RuntimeError(message)

    def run_viewer(self, config, command):
        result = subprocess.run(
            [str(self.viewer), "--config", str(config), "--output-format", "json", *command],
            capture_output=True, text=True, check=True, timeout=60,
        )
        return result.stdout

    @staticmethod
    def process(action):
        return action["process"]["process_id"]

    def starting_process(self, action):
        if action["kind"] != "command.invocation":
            return self.process(action)
        attrs = action["attributes"]
        self.require(attrs.get("process.parent.identity_state") == "observed",
                     f"command parent lacks observed identity: {action['action_id']}")
        return int(attrs["process.parent.id"])

    @staticmethod
    def ancestors(start, memberships):
        seen = set()
        while start is not None and start not in seen:
            seen.add(start)
            start = memberships.get(start)
        return seen

    def relationships(self, graph, trace_id, memberships):
        actions = {action["action_id"]: action for action in graph["actions"]}
        self.require(len(actions) == len(graph["actions"]), "duplicate action identities")
        counts = collections.Counter()
        command_edges = collections.defaultdict(set)
        for link in graph["links"]:
            if not link["valid"]:
                continue
            role = link["role"]
            if role not in self.COMMAND_ROLES and role != self.AGENT_ROLE:
                continue
            parent = actions[link["parent_action_id"]]
            child = actions[link["child_action_id"]]
            self.require(parent["trace_id_raw"] == child["trace_id_raw"] == link["trace_id_raw"] == trace_id,
                         f"cross-trace relationship: {role}")
            self.require(link["origin"] == "observed", f"non-live relationship: {role}")
            self.require(parent["action_id"] != child["action_id"], "self relationship")
            self.require(child["attributes"].get("process.parent.identity_state") != "conflict",
                         f"valid relationship on conflicted child: {role}")
            start = self.starting_process(child)
            if role == self.AGENT_ROLE:
                self.require(parent["kind"] == "agent.identity", "agent parent kind mismatch")
                self.require(child["kind"] in self.FILE_KINDS | {
                    "llm.call", "command.invocation", "process.fork_attempt"}, "agent child kind mismatch")
                self.require(self.process(parent) == start, "agent process ownership mismatch")
            else:
                self.require(parent["kind"] == "command.invocation", "command parent kind mismatch")
                self.require(child["kind"] in self.COMMAND_ROLES[role], "command child kind mismatch")
                self.require(self.process(parent) in self.ancestors(start, memberships),
                             f"command process is not an evidenced ancestor: {role}")
                if child["kind"] == "command.invocation":
                    command_edges[parent["action_id"]].add(child["action_id"])
            counts[role] += 1
        self.check_command_tree(command_edges)
        # Every actual LLM call must retain its command association in this workload.
        calls = {key for key, action in actions.items() if action["kind"] == "llm.call"}
        bound = {link["child_action_id"] for link in graph["links"]
                 if link["valid"] and link["role"] == "command.contains_llm_call"}
        self.require(calls and calls <= bound, "LLM call command coverage missing")
        return actions, dict(counts), sum(map(len, command_edges.values()))

    def check_command_tree(self, edges):
        remaining = set(edges) | {child for children in edges.values() for child in children}
        indegree = collections.Counter(child for children in edges.values() for child in children)
        self.require(all(count == 1 for count in indegree.values()), "multiple command parents")
        ready = [node for node in remaining if not indegree[node]]
        while ready:
            node = ready.pop()
            remaining.remove(node)
            for child in edges.get(node, ()):
                indegree[child] -= 1
                if not indegree[child]:
                    ready.append(child)
        self.require(not remaining, "command relationships contain a cycle")

    def otel(self, config, trace_id, graph, actions, path):
        self.run_viewer(config, ["export-otel", "--trace-id", str(trace_id), "--output", str(path)])
        document = json.loads(path.read_text())
        spans = [span for resource in document["resourceSpans"]
                 for scope in resource["scopeSpans"] for span in scope["spans"]]
        by_span = {}
        by_action = {}
        for span in spans:
            attrs = {entry["key"]: entry["value"] for entry in span["attributes"]}
            action_id = attrs["actrail.action.id"]["stringValue"]
            self.require(action_id in actions, "OTLP action missing from viewer")
            self.require(span["spanId"] not in by_span and action_id not in by_action,
                         "duplicate OTLP span/action identity")
            by_span[span["spanId"]] = action_id
            by_action[action_id] = span
        self.require(set(by_action) == set(actions), "OTLP/viewer action coverage differs")
        links = {(link["parent_action_id"], link["child_action_id"])
                 for link in graph["links"] if link["valid"]}
        required_parents = {link["child_action_id"] for link in graph["links"]
                            if link["valid"] and (link["role"] in self.COMMAND_ROLES
                                                  or link["role"] == self.AGENT_ROLE)}
        parents = 0
        for action_id, span in by_action.items():
            parent_id = span.get("parentSpanId")
            self.require(action_id not in required_parents or parent_id,
                         "OTLP omitted a tested live parent relationship")
            if parent_id:
                self.require(parent_id in by_span, "OTLP dangling parent span")
                self.require((by_span[parent_id], action_id) in links, "OTLP parent lacks valid action link")
                parents += 1
        self.require(parents > 0, "OTLP contains no parent relationships")
        return {"spans": len(spans), "parent_spans": parents, "path": str(path)}

    def trace(self, mode, trace_id):
        runtime = self.directory / f"runtime-{mode}"
        config = runtime / "actraild.conf"
        raw = self.run_viewer(config, ["actions", "--trace-id", str(trace_id)])
        (self.output / f"actions-{mode}-{trace_id}.json").write_text(raw)
        graph = json.loads(raw)
        database = runtime / "data/actrail.sqlite"
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
            memberships = dict(connection.execute(
                "SELECT process_id,inherited_from_process_id FROM memberships WHERE trace_id=?", (trace_id,)))
        actions, roles, edges = self.relationships(graph, trace_id, memberships)
        otel = self.otel(config, trace_id, graph, actions, self.output / f"otel-{mode}-{trace_id}.json")
        return {"mode": mode, "trace_id": trace_id, "actions": len(actions),
                "observed_roles": roles, "command_tree_edges": edges, "otel": otel}

    def run(self, functional=False):
        paths = (sorted(self.directory.glob("acceptance-[PC].json")) if functional
                 else [self.directory / "results.json"])
        self.require(paths, "no real-agent result files")
        samples = []
        for path in paths:
            source = json.loads(path.read_text())
            self.require(source["status"] == "passed", f"requires successful real-agent results: {path}")
            samples.extend(source["samples"])
        self.output.mkdir(exist_ok=False)
        report = {"status": "running", "viewer": str(self.viewer), "traces": [],
                  "not_exercised": ["web UI", "late events", "conflicting parent identity",
                                    "abnormal exit", "online OTLP consumer"]}
        try:
            traces = sorted({(sample["mode"], row[0]) for sample in samples
                             if sample["mode"] in ("P", "C") and (functional or sample["workload"].startswith("agent-"))
                             for row in sample["collection"]["traces"]})
            self.require(traces, "no real observed agent traces")
            for mode, trace_id in traces:
                report["traces"].append(self.trace(mode, trace_id))
            for mode in {mode for mode, _ in traces}:
                roles = collections.Counter()
                for trace in report["traces"]:
                    if trace["mode"] == mode:
                        roles.update(trace["observed_roles"])
                self.require(all(roles[role] for role in [*self.COMMAND_ROLES, self.AGENT_ROLE]),
                             f"{mode}: workload did not cover all five live relationship roles")
            report["status"] = "passed"
        except BaseException as error:
            report.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            (self.output / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--functional", action="store_true", help="read direct_acceptance real-agent results")
    args = parser.parse_args()
    LineageAcceptance(args.directory, args.bin_dir).run(args.functional)
