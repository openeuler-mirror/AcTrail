#!/usr/bin/env python3
"""Real file operations performed by the Bash tool of the traced Xiaoo process."""
import argparse
import errno
import json
import os
import subprocess
import time
from pathlib import Path
from argv_case import ArgvCase


class Workload:
    def __init__(self, work: Path, xiaoo: Path):
        self.work = work
        self.xiaoo = xiaoo

    def run(self) -> None:
        ancestors = []
        pid = os.getppid()
        while pid > 1:
            root = Path(f"/proc/{pid}")
            ancestors.append(str((root / "exe").resolve()))
            pid = int(next(line.split()[1] for line in (root / "status").read_text().splitlines() if line.startswith("PPid:")))
        if str(self.xiaoo) not in ancestors:
            raise RuntimeError("workload has no real Xiaoo ancestor")
        phase = (self.work / "phase").read_text().strip()
        names = [] if phase in ("commands", "argv", "reuse", "reuse-stopped") else (["allow", "deny", "grey", "grey", "grey", "grey", "read-deny", "read-allow"] if phase == "normal" else ["grey"])
        if phase in ("collision", "collision-swapped"):
            names = ["collision-primary", "collision-numeric"]
        results = ArgvCase(self.work).run() if phase == "argv" else []
        if phase in ("reuse", "reuse-stopped"):
            results = self.reuse()
        if phase == "commands":
            config = json.loads((self.work / "curl-config.json").read_text())
            for method in ("GET", "POST"):
                argv = [config["executable"], "--silent", "--show-error", "--max-time", "3", "-X", method, config["url"]]
                result = {"method": method, "argv": argv, "errno": 0}
                try:
                    with subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
                        stdout, stderr = child.communicate(timeout=5)
                        result.update(pid=child.pid, returncode=child.returncode, stdout=stdout.decode(), stderr=stderr.decode())
                except OSError as error:
                    result["errno"] = error.errno
                results.append(result)
        for name in names:
            started = time.monotonic()
            result = {"name": name, "errno": 0}
            descriptor = None
            try:
                descriptor = os.open(self.work / "protected" / name, os.O_RDONLY)
                result["opened"] = True
                result["content"] = os.read(descriptor, 128).decode()
            except OSError as error:
                result["errno"] = error.errno
            finally:
                if descriptor is not None:
                    os.close(descriptor)
            result["elapsed"] = time.monotonic() - started
            results.append(result)
        (self.work / f"results-{phase}.json").write_text(json.dumps({"pid": os.getpid(), "ancestors": ancestors, "results": results}))
        print(json.dumps(results))

    def reuse(self) -> list[dict]:
        # Alternate real exec calls in one trace. The first pair publishes the
        # service's executable decisions, and the following pairs reuse them.
        config = json.loads((self.work / "reuse-config.json").read_text())
        results = []
        for _ in range(config["rounds"]):
            for name, arguments in (("curl", ["--silent", "--max-time", "3", config["url"]]),
                                    ("cat", [config["file"]])):
                result = {"name": name, "errno": 0}
                try:
                    child = subprocess.run([config[name], *arguments], capture_output=True, text=True, timeout=5)
                    result.update(returncode=child.returncode, stdout=child.stdout, stderr=child.stderr)
                except OSError as error:
                    result["errno"] = error.errno
                results.append(result)
        return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--xiaoo", type=Path, required=True)
    args = parser.parse_args()
    Workload(args.work, args.xiaoo).run()
