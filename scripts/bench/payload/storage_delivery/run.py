"""Real agent acceptance for storage failures isolated from online analysis."""

import argparse
import json
import os
import shutil
import signal
import subprocess
import time
import tomllib
from pathlib import Path

from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.measurement import DaemonCpu
from scripts.bench.payload.runtime import CollectionRuntime

from .depth import ObservationDepthProbe
from .environment import DeliveryEnvironment


ROOT = Path(__file__).resolve().parents[4]


class StorageDeliveryAcceptance:
    def __init__(self, args):
        self.args = args
        self.output = args.output_dir.resolve()
        self.bins = args.bin_dir.resolve()
        self.binary = args.agent_bin.resolve()

    def run(self):
        self.output.mkdir(parents=True, exist_ok=False)
        report = {"status": "running", "scenarios": [],
                  "uncovered": ["TraceClosed write failure", "same poll batch co-residency"]}
        try:
            for name in self.args.scenarios:
                report["scenarios"].append(self.scenario(name))
            report["status"] = "passed"
        except BaseException as error:
            report.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            (self.output / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")

    def scenario(self, name):
        directory = self.output / name
        directory.mkdir()
        work = directory / "runtime"
        environment = DeliveryEnvironment(ROOT, self.bins, work, name)
        agent = AgentWorkload(ROOT, directory / "maas", self.binary, turns=4,
                              tpot_ms=3, timeout_seconds=60, input_bytes=128)
        # The actual shell tool waits, leaving time to inspect the living agent.
        agent._fixture["tool_command"] = "sleep 2; cat input.txt | tee result-{index}.txt"
        process = None
        collection = None
        result = {"scenario": name, "status": "running"}
        try:
            environment.prepare()
            effective = tomllib.loads(environment.operator_config.read_text())
            if effective["storage"]["sqlite"]["event_record_layout"] != "rows":
                raise RuntimeError("event fault requires freshly configured rows storage")
            pid = int((work / "run/actraild.pid").read_text())
            if Path(f"/proc/{pid}/exe").resolve() != self.bins / "actraild":
                raise RuntimeError("owned daemon does not match --bin-dir")
            collection = CollectionRuntime(work, self.bins, environment.patch,
                {"drain_timeout_seconds": 30, "poll_seconds": 0.02})
            collection.cpu = DaemonCpu(pid)
            collection.log = (work / "log/actraild.log").open(errors="replace")
            previous = collection.mark()
            environment.install_fault(previous)
            agent.start()
            task = directory / "agent"
            agent.prepare(task)
            agent.reset()
            command = collection.launch(agent.command(task))
            (directory / "command.json").write_text(json.dumps(command, indent=2) + "\n")
            probe = (ObservationDepthProbe(pid, self.binary, task / "xiaoo.toml")
                     if name == "agent-identity" else None)
            with (task / "stdout.log").open("wb") as stdout, (task / "stderr.log").open("wb") as stderr:
                process = subprocess.Popen(command, cwd=task, env=dict(os.environ, **agent.env),
                    stdout=stdout, stderr=stderr, start_new_session=True)
                deadline = time.monotonic() + 60
                while process.poll() is None:
                    collection.cpu.read_ms()
                    if probe and "observation_depth" not in result:
                        observed = probe.sample()
                        if observed:
                            result["observation_depth"] = observed
                    if time.monotonic() >= deadline:
                        raise TimeoutError("real agent exceeded 60 seconds")
                    time.sleep(0.05)
            if process.returncode:
                raise RuntimeError(f"real agent exited {process.returncode}")
            result["workload"] = agent.validate(task, task / "stdout.log")
            result["finalization"] = collection.drain(previous)
            traces = result["finalization"]["traces"]
            if len(traces) != 1:
                raise RuntimeError(f"expected one trace: {traces}")
            result["online"] = self.verify_online(environment)
            log = (work / "log/actraild.log").read_text(errors="replace")
            if "acceptance:storage_delivery_failure" not in log:
                raise RuntimeError("SQLite failure trigger did not observably fire")
            table, clause = (("events", "1") if name == "events"
                             else ("semantic_actions", "kind_code=119"))
            count = collection.query(f"SELECT COUNT(*) FROM {table} WHERE trace_id>? AND {clause}",
                                     (previous,))[0][0]
            if count:
                raise RuntimeError(f"faulted rows unexpectedly persisted: {count}")
            if probe and "observation_depth" not in result:
                raise RuntimeError("no generation-verified live agent depth=0 observation")
            result.update(status="passed", fault_log_verified=True, faulted_rows=count)
            return result
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            if process and process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
            agent.stop()
            if collection and collection.log:
                collection.log.close()
            try:
                environment.close()
            except BaseException as error:
                result.update(status="failed", cleanup_error=f"{type(error).__name__}: {error}")
                raise
            finally:
                (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")

    @staticmethod
    def verify_online(environment):
        deadline = time.monotonic() + 15
        counts = {}
        while time.monotonic() < deadline:
            identities = {}
            for span in environment.spans():
                attrs = environment.attributes(span)
                kind, identity = attrs.get("actrail.action.kind"), attrs.get("actrail.action.id")
                if kind and identity:
                    identities.setdefault(kind, set()).add(identity)
            counts = {kind: len(values) for kind, values in identities.items()}
            if counts.get("llm.request") == 4 and counts.get("llm.response") == 4:
                if counts.get("agent.identity", 0) >= 1:
                    return {"transport": "live OTLP/HTTP JSON", "unique_actions": counts}
            time.sleep(0.05)
        raise RuntimeError(f"incomplete online analysis under storage failure: {counts}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--agent-bin", type=Path,
                        default=Path(shutil.which("xiaoo") or "xiaoo"))
    parser.add_argument("--scenarios", nargs="+", choices=("events", "agent-identity"),
                        default=["events", "agent-identity"])
    StorageDeliveryAcceptance(parser.parse_args()).run()
