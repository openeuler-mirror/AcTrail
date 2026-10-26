"""Record uploaded file-context records during real-agent functional acceptance."""
import collections
import os
import re
import select
import signal
import subprocess


class ContextProbe:
    def __init__(self, event, pid, directory):
        self.directory = directory
        self.data = directory / "file-context.perf.data"
        self.log = (directory / "file-context.perf.log").open("w")
        control_read, control_write = os.pipe()
        ack_read, ack_write = os.pipe()
        try:
            self.process = subprocess.Popen(
                ["perf", "record", "-e", event, "-p", str(pid), "-o", str(self.data),
                 "--delay=-1", "--control", f"fd:{control_read},{ack_write}"],
                stdout=subprocess.DEVNULL, stderr=self.log,
                pass_fds=(control_read, ack_write),
            )
            os.write(control_write, b"enable\n")
            ack = os.read(ack_read, 64) if select.select([ack_read], [], [], 15)[0] else b""
            if ack.rstrip(b"\n\0") != b"ack":
                raise RuntimeError(f"context probe did not acknowledge readiness: {ack!r}")
            self.control_write, self.ack_read = control_write, ack_read
        except BaseException:
            if hasattr(self, "process") and self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=15)
            self.log.close()
            os.close(control_write)
            os.close(ack_read)
            raise
        finally:
            for fd in (control_read, ack_write):
                os.close(fd)

    def stop(self):
        try:
            if self.process.poll() is None:
                self.process.send_signal(signal.SIGINT)
            try:
                code = self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=15)
                raise
        finally:
            self.log.close()
            os.close(self.control_write)
            os.close(self.ack_read)
        if code not in (0, -signal.SIGINT):
            raise RuntimeError(f"context probe failed with status {code}")
        decoded = subprocess.run(
            ["perf", "script", "-i", str(self.data)],
            check=True, text=True, capture_output=True, timeout=30,
        ).stdout
        (self.directory / "file-context.txt").write_text(decoded)
        counts = collections.Counter()
        fcntl_commands = collections.Counter()
        fcntl_phases = collections.Counter()
        syscall_phases = collections.Counter()
        probe_counts = collections.Counter()
        for row in decoded.splitlines():
            probe = re.search(r"\b(actrail_\w+:\w+):", row)
            if probe:
                probe_counts[probe[1]] += 1
            match = re.search(r"\baux=(0x[0-9a-fA-F]+|[0-9]+)\b", row)
            if match:
                syscall = int(match[1], 0)
                counts[syscall] += 1
                phase = re.search(r"\bphase=(0x[0-9a-fA-F]+|[0-9]+)\b", row)
                if phase:
                    syscall_phases[f"{syscall}:{int(phase[1], 0)}"] += 1
                if syscall == 19:
                    for field, target in (("command", fcntl_commands), ("phase", fcntl_phases)):
                        value = re.search(rf"\b{field}=(0x[0-9a-fA-F]+|[0-9]+)\b", row)
                        if value:
                            target[int(value[1], 0)] += 1
        return {"syscall_counts": dict(sorted(counts.items())), "records": sum(counts.values()),
                "fcntl_commands": dict(sorted(fcntl_commands.items())),
                "fcntl_phases": dict(sorted(fcntl_phases.items())),
                "syscall_phases": dict(sorted(syscall_phases.items())),
                "probe_counts": dict(sorted(probe_counts.items())),
                "stdio_records": sum("actrail_mcp_gate:stdio_payload:" in row for row in decoded.splitlines())}
