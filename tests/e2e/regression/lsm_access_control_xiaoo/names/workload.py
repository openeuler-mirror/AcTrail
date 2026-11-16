#!/usr/bin/env python3
"""Issue rename, rename exchange and hard-link calls under a real agent."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from workload import AccessWorkload


class NameWorkload:
    phases = ("initial", "source", "target", "overwrite", "restore")
    cases = ("rename", "replace", "exchange", "link", "relative-rename", "relative-link", "same-inode")

    def __init__(self, arguments):
        self.arguments = arguments
        self.root = arguments.root
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.renameat2.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        self.libc.renameat2.restype = ctypes.c_int

    def run(self):
        processes = AccessWorkload(self.arguments)._processes()
        for phase in self.phases:
            (self.root / f"ready-{phase}").touch()
            self.wait(self.root / f"go-{phase}")
            operations = {}
            for name in self.cases:
                directory = self.root / name
                before = self.state(directory)
                try:
                    self.mutate(name, directory)
                    allowed = True
                    code = None
                except OSError as error:
                    allowed, code = False, error.errno
                after = self.state(directory)
                expected = before
                if allowed:
                    if name == "same-inode":
                        expected = before
                    elif name == "exchange":
                        expected = {"source": before["target"], "target": before["source"]}
                    elif "link" in name:
                        expected = {"source": before["source"], "target": before["source"]}
                    else:
                        expected = {"source": None, "target": before["source"]}
                if after != expected:
                    raise AssertionError(f"{phase}/{name}: unexpected names, contents or inodes: {before} -> {after}")
                operations[name] = {"allowed": allowed, "errno": code}
            destination = self.root / f"result-{phase}.json"
            temporary = destination.with_suffix(".tmp")
            temporary.write_text(json.dumps({"processes": processes, "operations": operations}))
            temporary.rename(destination)
        print("ACTRAIL_FILE_NAMES_DONE")

    def mutate(self, name, directory):
        source, target = directory / "source", directory / "target"
        if name.startswith("relative-"):
            descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
            try:
                action = os.link if name == "relative-link" else os.rename
                action("source", "target", src_dir_fd=descriptor, dst_dir_fd=descriptor)
            finally:
                os.close(descriptor)
        elif name == "link":
            os.link(source, target)
        elif name == "exchange":
            if self.libc.renameat2(-100, os.fsencode(source), -100, os.fsencode(target), 2):
                code = ctypes.get_errno()
                raise OSError(code, os.strerror(code))
        else:
            os.rename(source, target)

    @staticmethod
    def state(directory):
        return {name: (path.stat().st_ino, path.read_text()) if path.exists() else None
                for name in ("source", "target") for path in (directory / name,)}

    def wait(self, path):
        deadline = time.monotonic() + self.arguments.timeout
        while not path.exists():
            if time.monotonic() >= deadline:
                raise TimeoutError(f"parent did not release {path}")
            time.sleep(self.arguments.poll)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--xiaoo", type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--poll", type=float, required=True)
    NameWorkload(parser.parse_args()).run()
