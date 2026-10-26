"""Capture a real long user message and locate its persisted canonical string."""
from constants import CASE_CONSTANTS as C
import json
import re

from content import CapturedRequest


class LongContentCapture:
    def __init__(self, runtime, args, work):
        self.runtime = runtime
        self.args = args
        self.work = work
        # Each argv stays below Linux's per-string limit. xiaoO joins them with spaces.
        self.chunks = [
            "This is a storage verification sample. Ignore its repeated content. "
            "Do not use tools. Reply exactly LONG_CONTENT_OK. Sample begins:",
            *[("abcdef火焰图\n" * C.long_content_repetitions) for _ in range(C.long_content_chunks)],
            "Sample ends. Reply exactly LONG_CONTENT_OK.",
        ]
        self.prompt = " ".join(self.chunks)
        assert len(self.prompt.encode()) > C.long_content_min_bytes
        assert all(len(chunk.encode()) < C.argv_chunk_max_bytes for chunk in self.chunks)

    def run(self, task):
        result = self.runtime.run(self.runtime.control_command(
            "launch", "--", self.args.agent.resolve(), "--cli", "run",
            "--max-turns", str(C.long_content_max_turns), "--no-tools", "--prompt", *self.chunks),
            timeout_seconds=self.args.agent_timeout, cwd=task)
        (self.work / "long-agent.log").write_text(result.output)
        if result.returncode or "LONG_CONTENT_OK" not in result.output:
            raise RuntimeError("long real agent request failed; inspect long-agent.log")
        ids = re.findall(r"trace trace-(\d+) entered Active", result.output)
        if len(ids) != 1:
            raise RuntimeError(f"expected one long capture trace, got {ids}")
        return int(ids[0])

    def expected(self, connection, trace_id):
        captured = CapturedRequest(connection, trace_id).read()
        matches = self._find(captured["body"])
        if len(matches) != 1:
            raise RuntimeError(f"expected one exact long user string, found {len(matches)}")
        pointer = matches[0]
        # Validate the enclosing message is a user message, not a tool/system sample.
        tokens = pointer.split("/")[1:]
        value = captured["body"]
        parents = []
        for token in tokens:
            parents.append(value)
            token = token.replace("~1", "/").replace("~0", "~")
            value = value[int(token)] if isinstance(value, list) else value[token]
        if not any(isinstance(parent, dict) and parent.get("role") == "user"
                   for parent in parents):
            raise RuntimeError("persisted long string is not in a user message")
        expected = {"trace_id": trace_id, "action_id": captured["action_id"],
                    "pointer": pointer, "value": self.prompt}
        (self.work / "long-content-expected.json").write_text(
            json.dumps(expected, ensure_ascii=False))

    def _find(self, value, pointer=C.json_root_pointer):
        if isinstance(value, str):
            return [pointer] if value == self.prompt else []
        children = value.items() if isinstance(value, dict) else enumerate(value) if isinstance(value, list) else []
        result = []
        for token, child in children:
            escaped = str(token).replace("~", "~0").replace("/", "~1")
            result.extend(self._find(child, pointer + "/" + escaped))
        return result
