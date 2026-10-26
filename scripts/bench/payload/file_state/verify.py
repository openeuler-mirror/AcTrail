"""Compare persisted real file observations with the helper's execution evidence."""
import json
import os
import subprocess


class FileStateVerifier:
    def __init__(self, bins, runtime, directory, helper, *, mmap_only=False, no_files=False):
        self.bins, self.runtime, self.directory, self.helper = bins, runtime, directory, helper
        self.mmap_only = mmap_only
        self.no_files = no_files

    @staticmethod
    def require(condition, message):
        if not condition:
            raise RuntimeError(message)

    @classmethod
    def verify_uploads(cls, uploads):
        commands = {int(command) for command in uploads["fcntl_commands"]}
        cls.require({0, 1030}.issubset(commands) and commands <= {0, 2, 1030},
                    "fcntl upload omitted required commands or included commands without consumers")
        cls.require(set(uploads["fcntl_phases"]) == {2},
                    "fcntl must upload completed records only")
        phases = uploads["syscall_phases"]
        for syscall in (14, 15, 19):
            cls.require(phases.get(f"{syscall}:2", 0) > 0 and f"{syscall}:1" not in phases,
                        f"syscall {syscall} must upload completed records only")

    def verify(self, trace):
        result = subprocess.run([str(self.bins / "actrailviewer"), "--config",
            str(self.runtime / "actraild.conf"), "--output-format", "json", "events",
            "--trace-id", str(trace)], check=True, text=True, capture_output=True, timeout=60)
        (self.directory / "events.json").write_text(result.stdout)
        events = json.loads(result.stdout)["events"]
        proof = json.loads((self.directory / "agent/file-proof.json").read_text())
        self.require(proof["status"] == "passed", "file helper did not complete")
        if self.no_files:
            self.require(not any(event["variant"] == "file" for event in events),
                         "trace without file capability emitted file observations")
            return {"file_events": 0, "proof": proof}
        execs = [event for event in events if event["variant"] == "process"
                 and event["payload"].get("executable") == str(self.helper)
                 and event["payload"].get("operation") == "exec"]
        self.require(len(execs) == 2, "expected parent and child helper exec observations")
        parent = next(event["process"]["process_id"] for event in execs
                      if "\nchild\n" not in event["payload"]["metadata"].get("argv", ""))
        child = next(event["process"]["process_id"] for event in execs
                     if "\nchild\n" in event["payload"]["metadata"].get("argv", ""))
        files = [event for event in events if event["variant"] == "file"]
        a = str(self.directory / "agent/file-chain/A")
        b = str(self.directory / "agent/file-chain/B")
        parent_files = [event for event in files if event["process"]["process_id"] == parent]
        if self.mmap_only:
            self.require(any(event["payload"]["operation"] == "mmap_shared"
                             and event["payload"].get("path") == a for event in parent_files),
                         "mmap-only collection lost the file path")
            self.require(all(event["payload"]["operation"] == "mmap_shared" for event in files),
                         "mmap-only collection emitted other file operations")
            return {"parent_process_id": parent, "file_events": len(files), "proof": proof}
        closed_directory = next(event for event in files if event["process"]["process_id"] == child
                                and event["payload"]["operation"] == "open"
                                and (event["payload"].get("path") or "").endswith("closed-directory-child"))
        self.require(int(closed_directory["payload"]["result"]) < 0
                     and closed_directory["payload"]["metadata"].get("path_resolution") == "unresolved_relative"
                     and closed_directory["payload"].get("path") == "closed-directory-child",
                     "exec-closed directory FD retained a resolved path")
        self.require(not any(event["payload"]["operation"] in ("read", "readv", "write", "writev")
                             for event in files), "ordinary file I/O leaked per-syscall events")
        def summaries(pid, path):
            return [event["payload"]["io_summary"] for event in files
                    if event["process"]["process_id"] == pid and event["payload"].get("path") == path
                    and (event["payload"].get("io_summary") or {}).get("direction") == "write"]
        parent_a = summaries(parent, a)
        child_a = summaries(child, a)
        parent_b = summaries(parent, b)
        for items, expected, label in ((parent_a, 3, "parent A"), (child_a, 2, "fork/exec A"),
                                       (parent_b, 1, "reused FD B")):
            self.require(items and sum(item["operations"] for item in items) == expected
                         and sum(item["bytes"] for item in items) == expected,
                         f"typed write counts differ from real helper syscalls: {label}")
            self.require(all(item["errno"] == 0 and item["path_state"] == "resolved"
                             and item["target_kind"] == "regular_file" for item in items),
                         f"incorrect typed write identity/result: {label}")
        parent_tokens = {item["file_token"] for item in parent_a}
        child_tokens = {item["file_token"] for item in child_a}
        replacement_tokens = {item["file_token"] for item in parent_b}
        self.require(len(parent_tokens) == 2 and len(child_tokens) == 1
                     and child_tokens < parent_tokens and len(replacement_tokens) == 1
                     and not replacement_tokens.intersection(parent_tokens),
                     "dup/fork/exec, independent open or FD reuse corrupted file identity")
        shared_token = next(iter(child_tokens))
        self.require(sum(item["operations"] for item in parent_a if item["file_token"] == shared_token) == 2,
                     "parent source/dup contributions are missing or duplicated")
        action_result = subprocess.run([str(self.bins / "actrailviewer"), "--config",
            str(self.runtime / "actraild.conf"), "--output-format", "json", "actions",
            "--trace-id", str(trace)], check=True, text=True, capture_output=True, timeout=60)
        (self.directory / "actions.json").write_text(action_result.stdout)
        actions = json.loads(action_result.stdout)["actions"]
        for pid, path, expected in ((parent, a, 3), (child, a, 2), (parent, b, 1)):
            writes = [action for action in actions if action["kind"] == "file.write"
                      and action["process"]["process_id"] == pid and action["attributes"].get("file.path") == path]
            self.require(sum(int(action["attributes"]["file.write_count"]) for action in writes) == expected
                         and sum(int(action["attributes"]["file.bytes_written"]) for action in writes) == expected,
                         "typed write actions disagree with real syscall contributions")
        self.require(any(event["payload"]["operation"] == "mmap_shared" and event["payload"].get("path") == a
                         for event in parent_files), "shared writable mmap path missing")
        self.require(any(event["payload"]["operation"] == "mmap_shared"
                         and event["payload"].get("path") is None for event in parent_files),
                     "shared anonymous mmap missing")
        relative_open = [event for event in parent_files if event["payload"]["operation"] == "open"
                         and event["payload"].get("path") == a
                         and event["payload"].get("result") == proof["relative_fd"]
                         and event["payload"]["metadata"].get("syscall") == "openat2"]
        self.require(relative_open and all(int(event["payload"]["metadata"]["flags"]) == os.O_RDWR
                                          for event in relative_open),
                     "relative open path or non-creating intent incorrect")
        replacement_open = [event for event in parent_files if event["payload"]["operation"] == "open"
                            and event["payload"].get("path") == b
                            and event["payload"].get("result") == proof["replacement_fd"]
                            and int(event["payload"]["metadata"]["flags"]) == (os.O_CREAT | os.O_RDWR | os.O_TRUNC)]
        self.require(len(replacement_open) == 1, "replacement creating open fact missing")
        replacement_event = replacement_open[0]["event_id_raw"]
        self.require(any(action["kind"] == "file.modify" and action["process"]["process_id"] == parent
                         and action["attributes"].get("file.path") == b
                         and action["attributes"].get("file.open_intent") == "true"
                         and action["attributes"].get("file.change_kind") == "unknown"
                         and int(action["attributes"]["syscall.result"]) == proof["replacement_fd"]
                         and int(action["attributes"]["flags"]) == (os.O_CREAT | os.O_RDWR | os.O_TRUNC)
                         and any(item["kind"] == "event" and item["id"] == replacement_event
                                 for item in action["evidence"])
                         for action in actions), "replacement creating open intent missing")
        self.require(any(event["payload"]["metadata"].get("syscall") == "fcntl"
                         and int(event["payload"]["result"]) < 0 for event in parent_files),
                     "failed FD duplication observation missing")
        self.require(any(event["payload"]["operation"] == "rename" for event in parent_files), "rename missing")
        self.require(any(event["payload"]["operation"] == "unlink" for event in parent_files), "unlink missing")
        return {"parent_process_id": parent, "child_process_id": child, "file_events": len(files),
                "inherited_write_summaries": len(child_a), "fd_reuse_write_summaries": len(parent_b), "proof": proof}
