"""Real xiaoo MCP invocation against the existing local HTTPS MaaS."""
import json
import time
from pathlib import Path

from scripts.bench.payload.agent import AgentWorkload


class McpAgentWorkload(AgentWorkload):
    def __init__(self, *args, probe, spec, model_tool_name, **kwargs):
        super().__init__(*args, turns=2, input_bytes=128, **kwargs)
        self.probe, self.spec = probe, spec
        self.model_tool_name = model_tool_name
        self._fixture["prompt"] = f"Call the local MCP tool {model_tool_name} with marker {spec.marker}, then report completion."
        self._fixture["completion"] = "ACTRAIL_MCP_STATE_COMPLETE"

    def prepare(self, workdir: Path):
        super().prepare(workdir)
        command, arguments = self.probe.stdio_command(self.spec)
        (workdir / "mcp.json").write_text(json.dumps({"mcpServers": {
            self.spec.server_name: {"command": command, "args": arguments}
        }}))

    def command(self, workdir: Path):
        command = super().command(workdir)
        index = command.index("--tools")
        del command[index:index + 2]
        return command

    def _write_scenario(self, directory: Path):
        super()._write_scenario(directory)
        name = self._fixture["name"]
        scenario = directory / f"{name}.seq.json"
        document = json.loads(scenario.read_text())
        document["generators"][0]["response"]["blocks"] = [{
            "type": "tool_call", "name": self.model_tool_name,
            "arguments": {"marker": self.spec.marker},
        }]
        scenario.write_text(json.dumps(document))
        metadata = directory / f"{name}.meta.json"
        document = json.loads(metadata.read_text())
        document["tools"] = [self.model_tool_name]
        metadata.write_text(json.dumps(document))

    def validate(self, workdir: Path, output_path: Path):
        if self._fixture["completion"] not in output_path.read_text(errors="replace"):
            raise RuntimeError("real xiaoo did not reach the final MaaS response")
        self.probe.require_execution(self.spec)
        deadline = time.monotonic() + 1
        requests = []
        while time.monotonic() < deadline:
            with (self.work_dir / "maas.log").open("rb") as stream:
                stream.seek(self._log_offset)
                rows = stream.read().decode(errors="replace").splitlines()
            requests = [json.loads(row) for row in rows if row.startswith("{")]
            requests = [row for row in requests if row.get("event") == "local_maas_request"]
            if len(requests) >= 2:
                break
            time.sleep(0.01)
        if len(requests) != 2 or any(row["status"] != 200 for row in requests):
            raise RuntimeError(f"expected two successful HTTPS requests: {requests}")
        return {"llm_requests": 2, "mcp_executions": 1, "marker": self.spec.marker,
                "model_tool_name": self.model_tool_name, "transport": "https"}
