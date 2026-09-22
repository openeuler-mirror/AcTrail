"""Exercise provider retention through real OpenCode and local HTTPS MaaS."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tomllib
from pathlib import Path

from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.benchmark import BenchmarkLock, HERE, ROOT
from scripts.bench.payload.runtime import CollectionRuntime


class ProviderAgent(AgentWorkload):
    def _prepare_opencode(self, workdir: Path) -> None:
        super()._prepare_opencode(workdir)
        config = workdir / "opencode.json"
        document = json.loads(config.read_text())
        document["provider"]["bench"]["npm"] = "@ai-sdk/anthropic"
        config.write_text(json.dumps(document))

    def _write_scenario(self, directory: Path) -> None:
        super()._write_scenario(directory)
        path = directory / f"{self._fixture['name']}.seq.json"
        document = json.loads(path.read_text())
        for generator in document["generators"]:
            generator["response"]["blocks"].insert(0, {
                "type": "reasoning", "text": "Provider retention reasoning. " * 128,
            })
            generator["response"]["blocks"].insert(1, {
                "type": "message", "text": "Provider retention content. " * 128,
            })
        path.write_text(json.dumps(document))


class ProviderAcceptance:
    def __init__(self, out: Path, agent: Path):
        self.out = out.resolve()
        self.agent_binary = agent.resolve()
        self.bin_dir = ROOT / "target/release"
        self.settings = tomllib.loads((HERE / "configs/benchmark.toml").read_text())
        self.settings["agent_turns"] = 4

    def run(self) -> None:
        self.out.mkdir(parents=True, exist_ok=False)
        evidence = []
        agent = ProviderAgent(ROOT, self.out / "maas", self.agent_binary,
                              kind="opencode", turns=4, tpot_ms=0, input_bytes=65536,
                              timeout_seconds=30)
        with BenchmarkLock(Path("/run/lock/actrail-v2-regression.lock"), 5):
            CollectionRuntime.require_stopped()
            try:
                agent.start()
                for mode in ("P", "C", "content_off", "tools_off", "usage_off"):
                    print(f"[anthropic/{mode}]", flush=True)
                    evidence.append(self.scenario(agent, mode))
            finally:
                agent.stop()
        (self.out / "acceptance.json").write_text(json.dumps({
            "status": "passed", "provider": "anthropic", "scenarios": evidence,
        }, indent=2) + "\n")

    def scenario(self, agent: ProviderAgent, mode: str) -> dict:
        patch = self.out / f"{mode}.toml"
        content = (HERE / f"configs/{'P' if mode == 'P' else 'C'}.toml").read_text()
        if mode.endswith("_off"):
            field = {"content_off": "response_content", "tools_off": "tool_calls",
                     "usage_off": "usage"}[mode]
            content += f'\n[semantic_retention.l0_llm_call]\n{field} = "none"\n'
        patch.write_text(content)
        runtime = CollectionRuntime(self.out / f"r-{mode}", self.bin_dir, patch, self.settings)
        directory = self.out / mode
        try:
            runtime.start()
            agent.prepare(directory)
            agent.reset()
            mark = runtime.mark()
            with (directory / "stdout.log").open("wb") as stdout, (directory / "stderr.log").open("wb") as stderr:
                subprocess.run(runtime.launch(agent.command(directory)), cwd=directory,
                               env=dict(os.environ, **agent.env), stdout=stdout, stderr=stderr,
                               check=True, timeout=60)
            workload = agent.validate(directory, directory / "stdout.log")
            runtime.drain(mark)
            collection = runtime.evidence(mark, "agent", directory)
            trace_id = collection["traces"][0][0]
            output = subprocess.run([
                str(self.bin_dir / "actrailviewer"), "--config", str(runtime.config),
                "--output-format", "json", "actions", "--trace-id", str(trace_id),
            ], capture_output=True, text=True, check=True, timeout=30).stdout
            (directory / "actions.json").write_text(output)
            graph = json.loads(output)
            responses = [a for a in graph["actions"] if a["kind"] == "llm.response"]
            calls = [a for a in graph["actions"] if a["kind"] == "llm.tool_call"]
            if len(responses) != 4 or len(calls) != (0 if mode in ("P", "tools_off") else 3):
                raise RuntimeError(f"{mode}: provider response/tool coverage mismatch")
            for response in responses:
                attrs = response["attributes"]
                if (attrs.get("llm.response.provider_id") != "anthropic-messages"
                        or attrs.get("llm.response.done") != "true"
                        or response["status"] != "success" or response["completeness"] != "complete"):
                    raise RuntimeError(f"{mode}: provider completion evidence missing")
                for key in ("content_text", "reasoning_text"):
                    present = bool(attrs.get(f"llm.response.{key}"))
                    if present != (mode not in ("P", "content_off")):
                        raise RuntimeError(f"{mode}: {key} retention mismatch")
                if ("llm.response.completion_tokens" in attrs) != (mode not in ("P", "usage_off")):
                    raise RuntimeError(f"{mode}: usage retention mismatch")
                if int(attrs.get("llm.response.chunk_count", "0")) <= 0:
                    raise RuntimeError(f"{mode}: content-free chunk evidence missing")
            if sum(link["role"] == "llm.call.response" and link["valid"] for link in graph["links"]) != 4:
                raise RuntimeError(f"{mode}: response relationships missing")
            return {"mode": mode, "trace_id": trace_id, "workload": workload,
                    "responses": len(responses), "tools": len(calls)}
        finally:
            runtime.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--agent-bin", type=Path, default=Path("/usr/local/bin/opencode"))
    args = parser.parse_args()
    ProviderAcceptance(args.out, args.agent_bin).run()
