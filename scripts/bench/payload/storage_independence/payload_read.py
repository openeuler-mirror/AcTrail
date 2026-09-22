"""Real xiaoo session with direct local WAT loading and explicit payload reads."""

import argparse
import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.storage_delivery.environment import DeliveryEnvironment
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput


ROOT = Path(__file__).resolve().parents[4]
INSTANCE = "explicit-payload-read"


class PayloadRuntime(ActrailRuntime):
    def __init__(self, bins, config, patch, backend):
        super().__init__(ROOT, bins, 60, TestOutput(), config, patch,
                         clean_control_state=False)
        self.patch = patch
        self.backend = backend

    def prepare(self):
        with self.patch.open("a") as patch:
            patch.write(f'\n[storage]\nbackend = "{self.backend}"\n')
        return super().prepare()


class PayloadEnvironment(DeliveryEnvironment):
    def __init__(self, bins, work, backend):
        super().__init__(ROOT, bins, work, "payload-read")
        self.runtime = PayloadRuntime(bins, self.operator_config, self.patch, backend)
        self.backend = backend
        self.payload_loaded = False

    def prepare(self):
        super().prepare()
        manifest = ROOT / "examples/plugins/wasm-legacy/observation-payload-read/plugin.toml"
        fixture = self.config.work_dir / "payload-fixture"
        fixture.mkdir()
        wat = (manifest.parent / "payload-read.wat").read_text()
        difference = ["Omit the example's empty optional schema_ref; no configuration schema is requested."]
        if self.backend == "noop":
            terminal = "    (i64.const -1)\n  )\n)"
            if wat.count(terminal) != 1:
                raise RuntimeError("existing WAT terminal result changed")
            wat = wat.replace(terminal, "    (i64.const 0)\n  )\n)")
            difference.append(
                "NoOp accepts absent history: final consume result -1 becomes 0. "
                "All existing hostcalls remain; acceptance requires every call NotFound, "
                "zero returned bytes, zero Failed and zero Denied.")
        (fixture / "payload-read.wat").write_text(wat)
        (fixture / "plugin.toml").write_text(manifest.read_text().replace('schema_ref = ""\n', ""))
        (fixture / "difference.txt").write_text("\n".join(difference) + "\n")
        manifest = fixture / "plugin.toml"
        command = [str(self.config.bin_dir / "actraild"), "--config",
                   str(self.operator_config), "plugin", "load", "--manifest",
                   str(manifest), "--instance", INSTANCE, "--grant", "payload-read"]
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
        (self.config.work_dir / "payload-load.json").write_text(json.dumps({
            "command": command, "returncode": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}, indent=2) + "\n")
        result.check_returncode()
        self.payload_loaded = True

    def payload_status(self):
        document = self.api.catalog()
        (self.config.work_dir / "payload-status.json").write_text(
            json.dumps(document, indent=2) + "\n")
        return next(item for item in document["runtime_plugins"]
                    if item["instance_id"] == INSTANCE)

    def close(self):
        try:
            if self.payload_loaded:
                self.api.unload(INSTANCE)
                self.payload_loaded = False
        finally:
            super().close()


class PayloadReadAcceptance:
    def __init__(self, args):
        self.args = args
        self.work = args.output_dir.resolve()

    def run(self):
        self.work.mkdir(parents=True, exist_ok=False)
        bins = self.args.bin_dir.resolve()
        environment = PayloadEnvironment(bins, self.work / "runtime", self.args.backend)
        agent = AgentWorkload(ROOT, self.work / "maas", self.args.agent_bin.resolve(),
                              turns=2, tpot_ms=3, timeout_seconds=60)
        result = {"status": "running", "backend": self.args.backend,
                  "consumer": "existing legacy WAT, direct manifest load"}
        try:
            environment.prepare()
            agent.start()
            task = self.work / "agent"
            agent.prepare(task)
            agent.reset()
            command = [str(bins / "actrailctl"), "--config",
                       str(environment.operator_config), "launch", "--", *agent.command(task)]
            (self.work / "command.json").write_text(json.dumps(command, indent=2) + "\n")
            with (task / "stdout.log").open("wb") as stdout, (task / "stderr.log").open("wb") as stderr:
                process = subprocess.Popen(command, cwd=task, env=dict(os.environ, **agent.env),
                                           stdout=stdout, stderr=stderr, start_new_session=True)
                try:
                    status = process.wait(timeout=60)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=5)
                if status:
                    raise RuntimeError(f"real agent exited {status}")
            result["workload"] = agent.validate(task, task / "stdout.log")
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                plugin = environment.payload_status()
                metrics = plugin["hostcall_metrics"]["payload_read"]
                log = (environment.config.work_dir / "log/actraild.log").read_text(errors="replace")
                responses = {environment.attributes(span).get("actrail.action.id")
                             for span in environment.spans()
                             if environment.attributes(span).get("actrail.action.kind") == "llm.response"}
                if metrics["calls"] > 0 and "trace_finalization completed" in log and len(responses) == 2:
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("explicit read or online LLM finalization incomplete")
            if metrics["failed"] or metrics["denied"]:
                raise RuntimeError(f"unexpected broker failure/denial: {metrics}")
            if self.args.backend == "sqlite":
                if metrics["bytes"] == 0 or plugin["observed_records"] == 0:
                    raise RuntimeError(f"existing POST/OST fixture never succeeded: {plugin}")
            else:
                if metrics["not_found"] != metrics["calls"] or metrics["bytes"] != 0:
                    raise RuntimeError(f"NoOp did not return only NotFound: {metrics}")
                if environment.database.exists():
                    raise RuntimeError("NoOp created the main observation database")
            result.update(status="passed", plugin=plugin, online_llm_responses=len(responses))
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            agent.stop()
            try:
                environment.close()
            finally:
                (self.work / "acceptance.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--backend", choices=("sqlite", "noop"), required=True)
    parser.add_argument("--agent-bin", type=Path,
                        default=Path(shutil.which("xiaoo") or "xiaoo"))
    PayloadReadAcceptance(parser.parse_args()).run()
