"""Read real MCP lifecycle, relation, content and evidence through the viewer."""
import json
import sqlite3
import subprocess
import tomllib

from tests.v2.common.mcp_test_support.action_contract import McpActionContract


class ActionStateVerifier:
    KINDS = {"mcp.tool_call", "mcp.request", "mcp.response", "mcp.stdout", "mcp.stdin"}

    def __init__(self, bins, runtime, output, spec):
        self.bins, self.runtime, self.output, self.spec = bins, runtime, output, spec

    @staticmethod
    def require(condition, message):
        if not condition:
            raise RuntimeError(message)

    def actions(self, trace_id):
        result = subprocess.run([str(self.bins / "actrailviewer"), "--config",
            str(self.runtime / "actraild.conf"), "--output-format", "json", "actions",
            "--trace-id", str(trace_id)], check=True, text=True, capture_output=True, timeout=60)
        (self.output / "actions.json").write_text(result.stdout)
        return json.loads(result.stdout)

    def verify_ipc_capabilities(self, trace_id):
        config = tomllib.loads((self.runtime / "actraild.conf").read_text())
        capabilities = set(config["capture"]["capabilities"])
        result = subprocess.run([str(self.bins / "actrailviewer"), "--config",
            str(self.runtime / "actraild.conf"), "--output-format", "json", "events",
            "--trace-id", str(trace_id)], check=True, text=True, capture_output=True, timeout=60)
        (self.output / "events.json").write_text(result.stdout)
        events = json.loads(result.stdout)["events"]
        channels = {}
        for event in events:
            if event["variant"] == "ipc":
                channel = event["payload"]["channel"]
                channels[channel] = channels.get(channel, 0) + 1
        pipe_count = channels.get("pipe", 0) + channels.get("fifo", 0)
        if "ipc-pipe-fifo" in capabilities:
            self.require(pipe_count > 0, "requested ordinary pipe observation missing")
        else:
            self.require(pipe_count == 0, "ordinary pipe/FIFO observed without requested capability")
        if "ipc-unix-socket" not in capabilities:
            self.require(channels.get("unix_socket", 0) == 0,
                         "ordinary Unix socket observed without requested capability")
        return channels

    def verify_disabled(self, trace_id):
        graph = self.actions(trace_id)
        self.require(not any(action["kind"].startswith("mcp.") for action in graph["actions"]),
                     "MCP actions present with MCP disabled")
        with sqlite3.connect((self.runtime / "data/actrail.sqlite").as_uri() + "?mode=ro", uri=True) as database:
            kinds = dict(database.execute("SELECT kind_code,COUNT(*) FROM events WHERE trace_id=? GROUP BY kind_code",
                                          (trace_id,)).fetchall())
        channels = self.verify_ipc_capabilities(trace_id)
        config = tomllib.loads((self.runtime / "actraild.conf").read_text())
        if config["ebpf"]["file_path_capture_enabled"]:
            self.require(kinds.get(1, 0) > 0, "file observation missing")
        self.require(channels.get("stdio_bundle", 0) == 0,
                     "MCP bundle lifecycle present with MCP disabled")
        return {"mcp_actions": 0, "bundle_events": 0, "event_kind_counts": kinds,
                "ipc_channels": channels,
                "raw_payload_retention": config["semantic_retention"]["l4_payload"]["enabled"]}

    def verify(self, trace_id):
        channels = self.verify_ipc_capabilities(trace_id)
        graph = self.actions(trace_id)
        all_actions = {action["action_id"]: action for action in graph["actions"]}
        mcp = [action for action in graph["actions"] if action["kind"] in self.KINDS]
        self.require(len(mcp) == 5 and {action["kind"] for action in mcp} == self.KINDS,
                     "expected exactly five MCP invocation actions")
        by_kind = {action["kind"]: action for action in mcp}
        root = by_kind["mcp.tool_call"]
        contract = McpActionContract()
        for action in mcp:
            contract.require_terminal(action)
            contract.require_identity(action, self.spec)
            self.require(action["process"] == root["process"], "MCP server process identity differs")
            self.require(action["trace_id_raw"] == trace_id, "MCP trace identity differs")
        links = [link for link in graph["links"] if link["valid"]]
        expected = {
            "mcp.tool_call.request": (root, by_kind["mcp.request"]),
            "mcp.tool_call.response": (root, by_kind["mcp.response"]),
            "mcp.request.stdout": (by_kind["mcp.request"], by_kind["mcp.stdout"]),
            "mcp.response.stdin": (by_kind["mcp.response"], by_kind["mcp.stdin"]),
        }
        for role, (parent, child) in expected.items():
            found = [link for link in links if link["role"] == role
                     and link["parent_action_id"] == parent["action_id"]
                     and link["child_action_id"] == child["action_id"]]
            self.require(len(found) == 1 and found[0]["origin"] == "observed", f"missing unique {role}")
        parents = [link for link in links if link["role"] == "command.contains_mcp_tool_call"
                   and link["child_action_id"] == root["action_id"]]
        self.require(len(parents) == 1 and parents[0]["origin"] == "observed", "missing command/MCP association")
        self.require(all_actions[parents[0]["parent_action_id"]]["kind"] == "command.invocation",
                     "MCP association parent is not a command")
        # Classification belongs to the server's exec command; the tool-call
        # parent link can instead belong to the command that launched that server.
        server_commands = [action for action in graph["actions"]
                           if action["kind"] == "command.invocation" and action["process"] == root["process"]]
        self.require(len(server_commands) == 1, "MCP server command identity is not unique")
        server_command = server_commands[0]
        self.require(server_command["attributes"].get("invocation.kind") == "mcp", "server command was not classified as MCP")
        self.require(root["attributes"].get("mcp.execution.status") == "success", "MCP root lifecycle attribute missing")
        for suffix in ("request", "response", "stdout", "stdin"):
            contract.require_reference(root["attributes"], f"mcp.{suffix}.action_id", by_kind[f"mcp.{suffix}"])
        for action in mcp:
            if action is not root:
                contract.require_reference(action["attributes"], "mcp.tool_call.action_id", root)
        with sqlite3.connect((self.runtime / "data/actrail.sqlite").as_uri() + "?mode=ro", uri=True) as database:
            state = self.state(database, root, server_command, mcp)
            content = self.content(database, trace_id, by_kind)
        return {"mcp_actions": len(mcp), "mcp_links": 5, "root_action_id": root["action_id"],
                "ipc_channels": channels,
                "server_command": server_command["action_id"], "state": state, "content": content,
                "not_exercised": ["MCP timeout", "MCP error", "truncated MCP message", "online OTLP"]}

    def state(self, database, root, command, mcp):
        root_state = database.execute(
            "SELECT s.status_code,s.completeness_code,s.end_time FROM semantic_action_state s "
            "JOIN semantic_action_ids ids USING(action_key) WHERE ids.action_id=?", (root["action_id"],)).fetchall()
        self.require(len(root_state) == 1 and root_state[0][:2] == (202, 301) and root_state[0][2] is not None,
                     "independent MCP state is not success/complete/ended")
        command_state = database.execute(
            "SELECT s.command_invocation_kind FROM semantic_action_state s "
            "JOIN semantic_action_ids ids USING(action_key) WHERE ids.action_id=?", (command["action_id"],)).fetchall()
        self.require(command_state == [(2,)], "independent command classification differs")
        evidence_counts = {}
        config = tomllib.loads((self.runtime / "actraild.conf").read_text())
        retain_payload = config["semantic_retention"]["l4_payload"]["enabled"]
        kind_codes = {"event": 401, "payload_aggregate": 402, "payload_segment": 403}
        for action in [*mcp, command]:
            rows = database.execute(
                "SELECT e.kind_code,e.evidence_id,e.role FROM semantic_action_evidence e "
                "JOIN semantic_action_ids ids USING(action_key) WHERE ids.action_id=?", (action["action_id"],)).fetchall()
            expected = {(kind_codes[item["kind"]], item["id"], item["role"]) for item in action["evidence"]}
            self.require(set(rows) == expected, "viewer/state evidence membership differs")
            if retain_payload or action is command:
                self.require(expected, "retained MCP/command evidence is missing")
            evidence_counts[action["action_id"]] = len(rows)
        # The finalized view establishes persisted terminal facts, not a history
        # of every update. Request and response evidence must both remain present.
        root_ids = {(item["kind"], item["id"]) for item in root["evidence"]}
        for action in (next(a for a in mcp if a["kind"] == "mcp.request"),
                       next(a for a in mcp if a["kind"] == "mcp.response")):
            self.require({(item["kind"], item["id"]) for item in action["evidence"]} <= root_ids,
                         "MCP root lost request or response evidence")
        return {"root": root_state, "command": command_state, "evidence_counts": evidence_counts}

    def content(self, database, trace_id, by_kind):
        config = tomllib.loads((self.runtime / "actraild.conf").read_text())
        retention = config["semantic_retention"]["l0_mcp_call"]
        report = {}
        for direction in ("request", "response"):
            action = by_kind[f"mcp.{direction}"]
            rows = database.execute(
                "SELECT m.canonical_json,m.canonical_json_bytes FROM mcp_jsonrpc_action_refs r "
                "JOIN semantic_action_ids ids ON ids.action_key=r.action_key "
                "JOIN mcp_jsonrpc_messages m ON m.message_id=r.message_id "
                "WHERE r.trace_id=? AND ids.action_id=?", (trace_id, action["action_id"])).fetchall()
            enabled = retention[f"{direction}_content"] == "canonical_json"
            self.require(len(rows) == int(enabled), "canonical MCP retention differs from effective configuration")
            if enabled:
                raw, size = rows[0]
                raw = raw.encode() if isinstance(raw, str) else raw
                self.require(len(raw) == size, "canonical MCP content byte length differs")
                message = json.loads(raw)
                if direction == "request":
                    self.require(message["method"] == "tools/call" and message["params"]["name"] == self.spec.tool_name
                                 and message["params"]["arguments"] == {"marker": self.spec.marker}, "canonical MCP request differs")
                else:
                    self.require(message["result"]["content"] == [{"type": "text", "text": self.spec.marker}]
                                 and message["result"]["isError"] is False, "canonical MCP response differs")
                report[direction] = {"state": "retained", "bytes": size}
            else:
                report[direction] = {"state": "none"}
        return report
