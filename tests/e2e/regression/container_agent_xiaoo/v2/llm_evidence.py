from __future__ import annotations

import json
import re
import sqlite3
import subprocess
import tomllib
from contextlib import closing
from pathlib import Path

from .constants import CASE_CONSTANTS


class ContainerLlmEvidence:
    """Require captured outbound requests paired with each real agent's reply."""

    def __init__(self, runtime: Path, viewer: Path):
        self._runtime = runtime
        self._viewer = viewer.resolve()
        self._config = runtime / "operator.conf"

    def require(self) -> None:
        report: dict = {"status": "failed", "traces": []}
        try:
            outputs = sorted(self._runtime.glob("container-*.stdout"))
            if len(outputs) != 2:
                raise AssertionError("expected outputs from two container workloads")
            config = tomllib.loads(self._config.read_text())
            database = Path(config["storage"]["sqlite"]["path"])
            if not database.is_absolute():
                raise AssertionError("container storage path must be absolute")
            with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                for output in outputs:
                    text = output.read_text()
                    trace_ids = re.findall(r"trace trace-(\d+) entered Active", text)
                    if len(trace_ids) != 1:
                        raise AssertionError("container output must identify one trace")
                    # The original scenario has already verified this final answer
                    # against its own workload marker before returning successfully.
                    reply = text.strip().splitlines()[-1].strip()
                    report["traces"].append(self._require_trace(db, int(trace_ids[0]), reply))
            traces = report["traces"]
            if len({row["trace_id"] for row in traces}) != 2:
                raise AssertionError("container exchanges share a trace")
            if len({row["container_id"] for row in traces}) != 2:
                raise AssertionError("container exchanges share a container identity")
            if set(traces[0]["process_ids"]) & set(traces[1]["process_ids"]):
                raise AssertionError("container exchanges share internal process identities")
            report["status"] = "passed"
        except Exception as error:
            report["error"] = str(error)
            raise
        finally:
            (self._runtime / "llm-evidence.json").write_text(
                json.dumps(report, indent=2) + "\n", encoding="utf-8"
            )

    def _require_trace(self, db, trace_id: int, reply: str) -> dict:
        rows = db.execute(
            "SELECT trace_id,root_container_id FROM traces WHERE trace_id=?",
            (trace_id,),
        ).fetchall()
        if len(rows) != 1 or not rows[0][1]:
            raise AssertionError("workload must identify one container-owned trace")
        trace_id, container_id = rows[0]
        document = self._actions(trace_id)
        actions = {action["action_id"]: action for action in document["actions"]}
        links = {
            (link["parent_action_id"], link["role"], link["child_action_id"])
            for link in document["links"] if link.get("valid") is True
        }
        responses = {
            key for key, action in actions.items()
            if action["kind"] == "llm.response"
            and reply in action["attributes"].get("llm.response.content_text", "")
            and action["status"] == "success" and action["completeness"] == "complete"
            and action["attributes"].get("llm.response.done") == "true"
        }
        if not responses:
            raise AssertionError(f"trace-{trace_id} lacks a complete task reply")
        exchanges = []
        process_ids = set()
        for response_id in sorted(responses):
            pairs = [
                (call_id, request_id)
                for call_id, role, child in links
                if role == "llm.call.response" and child == response_id
                and actions.get(call_id, {}).get("kind") == "llm.call"
                for parent, request_role, request_id in links
                if parent == call_id and request_role == "llm.call.request"
                and actions.get(request_id, {}).get("kind") == "llm.request"
            ]
            if len(pairs) != 1:
                raise AssertionError("task reply must have one valid call/request pairing")
            call_id, request_id = pairs[0]
            request = actions[request_id]
            if request["status"] != "success" or request["completeness"] != "complete":
                raise AssertionError(f"trace-{trace_id} contains an incomplete outbound request")
            body = self._body(db, trace_id, request_id)
            if not isinstance(body, dict) or not isinstance(body.get("model"), str) or not body["model"]:
                raise AssertionError("request must contain its model")
            messages = body.get("messages")
            if not isinstance(messages, list) or not messages:
                raise AssertionError("request must contain structured messages")
            if any(not isinstance(message, dict) or "role" not in message for message in messages):
                raise AssertionError("request messages must retain their roles")
            if not any(message["role"] == "user" and message.get("content") for message in messages):
                raise AssertionError("request must retain nonempty user input")
            serialized = json.dumps(body, ensure_ascii=False)
            http = [
                actions[child] for parent, role, child in links
                if parent == request_id and role == "llm.request.http_message"
                and child in actions and actions[child]["kind"] == "http.message"
                and actions[child]["attributes"].get("method") == "POST"
                and actions[child]["attributes"].get("direction") == "outbound"
            ]
            if len(http) != 1:
                raise AssertionError("task request must link to its outbound HTTP POST")
            for action_id in (request_id, response_id, http[0]["action_id"]):
                row = db.execute(
                    "SELECT a.process_id FROM semantic_actions a JOIN semantic_action_ids ids USING(action_key) "
                    "JOIN memberships m ON m.trace_id=a.trace_id AND m.process_id=a.process_id "
                    "WHERE a.trace_id=? AND ids.action_id=?", (trace_id, action_id),
                ).fetchone()
                if row is None:
                    raise AssertionError("exchange action has no trace process membership")
                process_ids.add(row[0])
            exchanges.append({
                "request_id": request_id, "call_id": call_id, "response_id": response_id,
                "http_action_id": http[0]["action_id"],
                "stream_key": http[0]["attributes"].get("stream_key"),
                "body_bytes": len(serialized.encode()), "message_count": len(messages),
                "user_input_present": True,
                "valid_call_pair": True,
            })
        if not exchanges:
            raise AssertionError(f"trace-{trace_id} has no complete outbound task exchange")
        return {"trace_id": trace_id, "container_id": container_id,
                "process_ids": sorted(process_ids), "exchanges": exchanges}

    def _actions(self, trace_id: int) -> dict:
        result = subprocess.run(
            [str(self._viewer), "--config", str(self._config), "--output-format", "json",
             "actions", "--trace-id", f"trace-{trace_id}"],
            capture_output=True, text=True,
            timeout=CASE_CONSTANTS.viewer_timeout_seconds, check=False,
        )
        if result.returncode:
            diagnostic = result.stderr[-CASE_CONSTANTS.viewer_diagnostic_tail_characters:]
            raise RuntimeError(f"viewer failed for trace-{trace_id}: {diagnostic}")
        return json.loads(result.stdout)

    def _body(self, db, trace_id: int, action_id: str):
        row = db.execute(
            "SELECT m.manifest_id,m.skeleton_json,m.format_version FROM llm_request_manifests m "
            "JOIN semantic_action_ids ids USING(action_key) WHERE m.trace_id=? AND ids.action_id=?",
            (trace_id, action_id),
        ).fetchone()
        if row is None or row[2] != 2:
            raise AssertionError("request must retain its version-2 canonical JSON manifest")
        blocks = {}
        for ordinal, owner, size, raw in db.execute(
            "SELECT r.ordinal,b.trace_id,b.uncompressed_bytes,b.encoded_bytes "
            "FROM llm_request_block_refs r JOIN llm_request_blocks b USING(block_id) "
            "WHERE r.manifest_id=? ORDER BY r.ordinal", (row[0],),
        ):
            if owner != trace_id or len(raw) != size or ordinal != len(blocks):
                raise AssertionError("request block identity, length, or ordinal is invalid")
            blocks[ordinal] = json.loads(raw)
        return self._hydrate(json.loads(row[1]), blocks)

    def _hydrate(self, value, blocks):
        if isinstance(value, dict):
            if set(value) == {"$actrail_llm_block"}:
                return blocks[value["$actrail_llm_block"]]
            return {key: self._hydrate(child, blocks) for key, child in value.items()}
        if isinstance(value, list):
            return [self._hydrate(child, blocks) for child in value]
        return value
