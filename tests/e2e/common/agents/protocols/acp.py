"""Handle the standard ACP conversation protocol inside agent adapters."""
from __future__ import annotations

import json
from pathlib import Path

from ..base import AgentResult
from ..process import AgentProcess


class AcpSession:
    """ACP 会话拥有协议 ID，进程生命周期仍由 AgentProcess 管理。"""

    def __init__(self, process: AgentProcess, cwd: Path, *, protocol_version: int | str):
        self._process = process
        self._cwd = cwd.resolve()
        self._session_id: str | None = None
        self._next_id = 0
        self._protocol_version = protocol_version

    def run(self, prompt: str) -> AgentResult:
        if self._session_id is None:
            self._request("initialize", {"protocolVersion": self._protocol_version})
            result, _ = self._request("session/new", {
                "cwd": str(self._cwd), "mcpServers": [],
            })
            self._session_id = result["sessionId"]
        result, output = self._request("session/prompt", {
            "sessionId": self._session_id, "prompt": [{"type": "text", "text": prompt}],
        })
        success = result.get("stopReason") == "end_turn"
        return AgentResult(output, success, None if success else str(result.get("stopReason")))

    def _request(self, method: str, params: dict) -> tuple[dict, str]:
        self._next_id += 1
        identifier = self._next_id
        self._write({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
        output = []
        for line in self._process.process.stdout:
            event = json.loads(line)
            if event.get("method") == "session/request_permission":
                # 本客户端没有交互审批界面，通过 ACP 明确取消请求。
                self._write({"jsonrpc": "2.0", "id": event["id"], "result": {
                    "outcome": {"outcome": "cancelled"},
                }})
            elif event.get("method") == "session/update":
                data = event.get("params", {})
                update = data.get("update", {})
                if (data.get("sessionId") == self._session_id
                        and update.get("sessionUpdate") == "agent_message_chunk"):
                    content = update.get("content", {})
                    if content.get("type") == "text":
                        output.append(content.get("text", ""))
            elif event.get("id") == identifier and "method" not in event:
                if "error" in event:
                    raise RuntimeError(event["error"]["message"])
                return event["result"], "".join(output)
            elif "id" in event and "method" in event:
                self._write({"jsonrpc": "2.0", "id": event["id"], "error": {
                    "code": -32601, "message": "Client capability is not supported",
                }})
        raise RuntimeError("Agent exited before completing the ACP request")

    def _write(self, event: dict) -> None:
        self._process.process.stdin.write(json.dumps(event) + "\n")
        self._process.process.stdin.flush()
