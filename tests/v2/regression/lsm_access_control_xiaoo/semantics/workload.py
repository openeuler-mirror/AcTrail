#!/usr/bin/env python3
"""Exercise file permission semantics through real operating-system calls."""
import argparse
import ctypes
import json
import mmap
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from workload import AccessWorkload
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "continued"))
from mapping import MappingOperations
from mutations import FileMutations
from truncation import TruncationOperations
from resources import FileResources


class FileSemanticsWorkload:
    def __init__(self, arguments):
        self.arguments = arguments
        self.root = arguments.root

    def run(self):
        processes = AccessWorkload(self.arguments)._processes()
        descriptor = os.open(self.root / "protected", os.O_RDWR)
        maps = MappingOperations({"protected": descriptor})
        mutations = FileMutations(self.root, descriptor)
        truncation = TruncationOperations(self.root)
        private = maps.libc.mmap(None, maps.size, mmap.PROT_READ, mmap.MAP_PRIVATE, descriptor, 0)
        if private == ctypes.c_void_p(-1).value:
            maps.raise_errno()
        try:
            for phase in ("empty", "open", "write", "read", "delete", "attr", "restore"):
                (self.root / f"ready-{phase}").touch()
                self.wait(self.root / f"go-{phase}")
                operations = {}
                for label, action in (
                    ("read", lambda: os.pread(descriptor, 1, 0).decode()),
                    ("write", lambda: os.pwrite(descriptor, phase.encode(), 0)),
                    ("open-read", lambda: self.open_file(os.O_RDONLY)),
                    ("open-write", lambda: self.open_file(os.O_WRONLY)),
                    ("open-read-write", lambda: self.open_file(os.O_RDWR)),
                    ("private-write", lambda: self.private_map(descriptor, maps)),
                    ("private-mprotect-write", lambda: self.private_protect(private, maps)),
                ):
                    try:
                        operations[label] = {"allowed": True, "value": action()}
                    except OSError as error:
                        operations[label] = {"allowed": False, "errno": error.errno}
                maps.run(operations)
                mutations.run(operations)
                truncation.run(operations)
                destination = self.root / f"result-{phase}.json"
                temporary = destination.with_suffix(".tmp")
                document = {"processes": processes, "operations": operations}
                if self.arguments.resources:
                    document["resources"] = FileResources(self.root).run()
                temporary.write_text(json.dumps(document))
                temporary.rename(destination)
            print("ACTRAIL_FILE_SEMANTICS_DONE")
        finally:
            maps.close()
            mutations.close()
            truncation.close()
            maps.libc.munmap(private, maps.size)
            os.close(descriptor)

    def open_file(self, flags):
        descriptor = os.open(self.root / "protected", flags)
        os.close(descriptor)

    @staticmethod
    def private_protect(address, maps):
        maps.protect(address, 0)
        maps.protect(address, mmap.PROT_READ | mmap.PROT_WRITE)
        ctypes.c_char.from_address(address).value = b"!"
        assert ctypes.c_char.from_address(address).value == b"!"

    @staticmethod
    def private_map(descriptor, maps):
        address = maps.libc.mmap(None, maps.size, mmap.PROT_READ | mmap.PROT_WRITE,
                                mmap.MAP_PRIVATE, descriptor, 0)
        if address == ctypes.c_void_p(-1).value:
            maps.raise_errno()
        try:
            ctypes.c_char.from_address(address).value = b"!"
            assert ctypes.c_char.from_address(address).value == b"!"
        finally:
            maps.libc.munmap(address, maps.size)

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
    parser.add_argument("--resources", action="store_true")
    parser.add_argument("--poll", type=float, required=True)
    FileSemanticsWorkload(parser.parse_args()).run()
