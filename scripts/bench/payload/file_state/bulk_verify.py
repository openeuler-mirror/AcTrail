"""Verify real typed file summaries, raw retention and batch path membership."""
import json
import sqlite3
import subprocess
import tomllib

from .verify import FileStateVerifier


class BulkReadVerifier:
    require = staticmethod(FileStateVerifier.require)

    def __init__(self, bins, runtime, directory, helper, retention):
        self.bins, self.runtime, self.directory = bins, runtime, directory
        self.helper, self.retention = helper, retention

    def read_view(self, kind, trace):
        result = subprocess.run([str(self.bins / "actrailviewer"), "--config",
            str(self.runtime / "actraild.conf"), "--output-format", "json", kind,
            "--trace-id", str(trace)], check=True, text=True, capture_output=True, timeout=60)
        (self.directory / f"{kind}.json").write_text(result.stdout)
        return json.loads(result.stdout)[kind]

    def path_members(self, database, trace, path_set):
        return {row[0] for row in database.execute(
            "SELECT p.path_text FROM file_path_set_chunk_refs r "
            "JOIN file_path_set_chunks c ON c.trace_id=r.trace_id AND c.chunk_id=r.chunk_id "
            "JOIN file_paths p ON p.trace_id=c.trace_id "
            "AND instr(',' || c.encoded_sorted_path_ids || ',', ',' || p.path_id || ',') > 0 "
            "WHERE r.trace_id=? AND r.path_set_id=?", (trace, path_set))}

    def verify(self, trace):
        config = tomllib.loads((self.runtime / "actraild.conf").read_text())
        file_config = config["file_observation"]
        bulk = file_config["bulk_read"]
        self.require(bulk["enabled"] and bulk["mode"] == "path_set"
                     and bulk["raw_event_retention"] == self.retention,
                     "bulk summary verification configuration mismatch")
        demand = file_config["collection"]["read"]
        self.require(demand["counts"] and demand["bytes"] and demand["errors"]
                     and file_config["collection"]["fd_mutations"],
                     "fixture requires read counts/bytes/errors and failed-open observations")
        proof = json.loads((self.directory / "agent/file-proof.json").read_text())
        self.require(proof["status"] == "passed" and proof["reads"] == 3 and proof["errors"] == 2,
                     "real bulk reader did not complete")
        events = self.read_view("events", trace)
        self.require(len({event["event_id_raw"] for event in events}) == len(events), "duplicate event IDs")
        execs = [event for event in events if event["variant"] == "process"
                 and event["payload"].get("operation") == "exec"
                 and event["payload"].get("executable") == str(self.helper)]
        self.require(len(execs) == 1, "expected one bulk helper exec")
        process = execs[0]["process"]
        files = [event for event in events if event["variant"] == "file" and event["process"] == process]
        root = self.directory / "agent/bulk-chain"
        targets = {str(root / name) for name in ("A", "B", "C")}
        self.require(not any(event["payload"]["operation"] in ("read", "readv", "write", "writev")
                             for event in files), "file I/O leaked per-syscall events")
        for name in ("before", "after"):
            failed = [event for event in files if event["payload"].get("path") == str(root / name)
                      and event["payload"]["operation"] == "open"]
            self.require(len(failed) == 1 and failed[0]["payload"]["result"] == -2,
                         f"missing actual failed open ENOENT: {name}")
        raw_reads = [event for event in files if (event["payload"].get("io_summary") or {}).get("direction") == "read"
                     and event["payload"].get("path") in targets]
        if self.retention == "full":
            tokens = set()
            for path in targets:
                snapshots = [event["payload"]["io_summary"] for event in raw_reads if event["payload"]["path"] == path]
                self.require(snapshots and sum(item["operations"] for item in snapshots) == 1
                             and sum(item["bytes"] for item in snapshots) == 1,
                             f"typed deltas differ from actual one-byte read: {path}")
                self.require(all(item["errno"] == 0 and item["path_state"] == "resolved"
                                 and item["target_kind"] == "regular_file" for item in snapshots),
                             "incorrect typed file identity/result")
                identities = {item["file_token"] for item in snapshots}
                self.require(len(identities) == 1, "one open lifetime changed file token")
                tokens.update(identities)
            self.require(len(tokens) == 3, "distinct live file identities reused a token")
        else:
            self.require(not raw_reads, "errors-only retained successful file I/O summaries")
        actions = self.read_view("actions", trace)
        summaries = [action for action in actions if action["kind"] == "file.bulk_read" and action["process"] == process]
        self.require(summaries, "missing actual bulk summary actions")
        members = set()
        relevant = []
        with sqlite3.connect((self.runtime / "data/actrail.sqlite").as_uri() + "?mode=ro", uri=True) as database:
            for action in summaries:
                attrs = action["attributes"]
                path_set = attrs.get("file.bulk_read.path_set_id")
                if not path_set:
                    continue
                current = self.path_members(database, trace, path_set)
                if not current.intersection(targets):
                    continue
                self.require(attrs.get("file.interval_basis") == "collector_batch"
                             and int(attrs["file.bulk_read.unique_path_count"]) == len(current),
                             "batch summary does not describe its stored members")
                members.update(current.intersection(targets))
                relevant.append(action)
        self.require(members == targets, "stored batch path membership omitted actual reads")
        self.require(sum(int(action["attributes"]["file.bulk_read.read_count"]) for action in relevant) >= 3
                     and sum(int(action["attributes"]["file.bytes_read"]) for action in relevant) >= 3,
                     "bulk statistics omitted helper reads")
        self.require(not any(action["kind"] == "file.read" and action["process"] == process
                             and action["attributes"].get("file.path") in targets for action in actions),
                     "bulk input also produced duplicate individual read actions")
        return {"retention": self.retention, "summary_actions": [action["action_id"] for action in relevant],
                "stored_paths": sorted(members), "raw_read_summaries": len(raw_reads), "proof": proof}
