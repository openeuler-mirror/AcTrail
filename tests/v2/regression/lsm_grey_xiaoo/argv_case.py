"""Compare authorized argv with arguments received by real exec targets under Xiaoo."""
import errno
import json
import shutil
import subprocess
from pathlib import Path


class ArgvCase:
    def __init__(self, work: Path):
        self.work = work
        self.launcher = work / "argv-launcher"
        self.target = work / "argv-target"

    def prepare(self) -> str:
        subprocess.run(["cc", "-O2", "-Wall", "-Wextra", "-Werror", "-pthread",
                        str(Path(__file__).with_name("argv_workload.c")), "-o", str(self.launcher)], check=True)
        shutil.copy2(self.launcher, self.target)
        return f"argv-grey gray sync-plugin wasm.security-service exec {self.target} priority 0\n"

    def run(self) -> list[dict]:
        results = []
        for name, rounds, length, count, racing in (
            ("race", 32, 200, 4, 1), ("limit", 2, 255, 16, 0),
            ("long", 2, 256, 4, 0), ("many", 2, 200, 17, 0),
        ):
            result = subprocess.run([str(self.launcher), str(self.target), str(rounds),
                                     str(length), str(count), str(racing)],
                                    text=True, capture_output=True, timeout=20, check=True)
            results.extend({**json.loads(line), "case": name} for line in result.stdout.splitlines())
        return results

    def verify(self, document: dict, requests: list[dict]) -> None:
        results = document["results"]
        if len(results) != 38 or len(requests) != 34:
            raise AssertionError(f"unexpected argv results or requests: {len(results)}, {len(requests)}")
        by_pid = {item["pid"]: item for item in requests}
        if len(by_pid) != 34:
            raise AssertionError("argv requests must identify distinct exec children")
        for result in results:
            request = by_pid.get(result["pid"])
            if result["case"] in ("long", "many"):
                if result["errno"] != errno.EACCES or request is not None:
                    raise AssertionError(f"incomplete argv was authorized despite allow fallback: {result}")
                continue
            if result["errno"] or request is None:
                raise AssertionError(f"complete argv execution failed: {result}")
            dirfd, flags = (-100, 0) if result["syscall"] == "execveat" else (None, None)
            if (request["execveat_dirfd"], request["execveat_flags"]) != (dirfd, flags):
                raise AssertionError(f"incorrect execveat context: {request}, {result}")
            expected = [str(self.target), "--argv-probe", result["payload"], result["syscall"]]
            expected += ["filler"] * (result["argc"] - 4)
            if request["argv"] != expected or request["verdict"] != "allow":
                raise AssertionError(f"authorized argv differs from executed argv: {request}, {result}")
            if request["trace_id"] != str(document["trace_id"]) or request["executable"] != str(self.target):
                raise AssertionError(f"incorrect argv execution identity: {request}")
