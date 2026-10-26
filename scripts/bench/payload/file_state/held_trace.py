"""Keep an owned file trace active while another trace requests fewer capabilities."""
import shlex
import subprocess
import time
import tomllib

from scripts.bench.payload.action_state.context_probe import ContextProbe


class HeldFileTrace:
    def __init__(self, collection, pid, event, directory):
        self.collection, self.pid, self.event = collection, pid, event
        self.directory = directory
        self.directory.mkdir()
        self.process = None
        self.log = None
        self.baseline = None

    def start(self):
        config = tomllib.loads(self.collection.config.read_text())
        tls = config["payload"]["tls"]
        if tls["direct_startup_discovery_enabled"] or tls["direct_dynamic_discovery_enabled"]:
            raise RuntimeError("mixed trace verification requires TLS discovery explicitly disabled")
        group = self.event.split(":", 1)[0]
        self.expected_probes = {f"{group}:{name}" for name in (
            "context", "tracker_seed", "tracker_exec", "tracker_inherit", "tracker_record")}
        registered = subprocess.run(["perf", "probe", "--list"], check=True,
            capture_output=True, text=True, timeout=15).stdout
        if self.event != f"{group}:*" or not self.expected_probes.issubset(
                {line.split()[0] for line in registered.splitlines() if line.strip()}):
            raise RuntimeError("mixed trace verification requires all five context/tracker probes")
        marker = self.directory / "ready"
        self.log = (self.directory / "launch.log").open("w")
        probe = ContextProbe(self.event, self.pid, self.directory)
        try:
            command = self.collection.launch([
                "/bin/sh", "-c", f"printf ready > {shlex.quote(str(marker))} && read -r done",
            ])
            self.process = subprocess.Popen(command, stdin=subprocess.PIPE,
                stdout=self.log, stderr=self.log, text=True)
            deadline = time.monotonic() + 30
            while not marker.exists():
                if self.process.poll() is not None:
                    raise RuntimeError("file trace holder exited before readiness")
                if time.monotonic() >= deadline:
                    raise RuntimeError("file trace holder did not become ready")
                time.sleep(0.01)
        finally:
            self.baseline = probe.stop()

    def verify(self, uploads):
        if self.process.poll() is not None:
            raise RuntimeError("file trace holder did not remain active")
        baseline_calls = sum(count for event, count in self.baseline["probe_counts"].items()
                             if ":tracker_" in event)
        target_calls = sum(count for event, count in uploads["probe_counts"].items()
                           if ":tracker_" in event)
        if not baseline_calls or not uploads["records"] or target_calls:
            raise RuntimeError(f"mixed trace probe mismatch: baseline tracker calls={baseline_calls}, "
                               f"target context records={uploads['records']}, tracker calls={target_calls}")
        return {"baseline": self.baseline, "target_tracker_calls": target_calls,
                "registered_probes": sorted(self.expected_probes),
                "holder_active": True, "tls_plaintext_coverage": "discovery explicitly disabled"}

    def stop(self):
        try:
            if self.process:
                if self.process.stdin and not self.process.stdin.closed:
                    try:
                        self.process.stdin.write("done\n")
                        self.process.stdin.flush()
                    except BrokenPipeError:
                        pass
                    self.process.stdin.close()
                try:
                    code = self.process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    self.process.terminate()
                    try:
                        self.process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        self.process.kill()
                        self.process.wait(timeout=5)
                    raise
                if code:
                    raise RuntimeError(f"file trace holder exited with {code}")
        finally:
            if self.log:
                self.log.close()
