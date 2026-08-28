#!/usr/bin/env python3
"""Deterministic operation workloads used by the detail benchmark."""

from __future__ import annotations

import argparse
import http.client
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence
from urllib.parse import urlsplit


OPERATION_NAMES = (
    "file-write",
    "file-read",
    "bash-light",
    "network",
    "bash-heavy",
    "mixed",
)


@dataclass(frozen=True, slots=True)
class WorkloadCase:
    name: str
    label: str
    count: int


class WorkloadSuite:
    def __init__(
        self,
        root: Path,
        *,
        file_bytes: int,
        compiler: str,
        network_url: str,
    ) -> None:
        self.root = root
        self.file_bytes = file_bytes
        self.compiler = compiler
        self.network_url = network_url
        self._payload = b"a" * file_bytes

    def prepare(self, case: WorkloadCase) -> None:
        case_root = self.root / case.name
        shutil.rmtree(case_root, ignore_errors=True)
        case_root.mkdir(parents=True)
        if case.name == "file-read":
            self._create_read_files(case_root, case.count)
        elif case.name == "bash-heavy":
            self._create_compile_source(case_root)
        elif case.name == "mixed":
            self._create_read_files(case_root, case.count)
            self._create_compile_source(case_root)

    def command(self, case: WorkloadCase, elapsed_path: Path) -> list[str]:
        return [
            str(Path(__file__).resolve()),
            "--operation",
            case.name,
            "--count",
            str(case.count),
            "--root",
            str(self.root / case.name),
            "--file-bytes",
            str(self.file_bytes),
            "--compiler",
            self.compiler,
            "--network-url",
            self.network_url,
            "--elapsed-path",
            str(elapsed_path),
        ]

    def run(self, operation: str, count: int, root: Path) -> None:
        methods = {
            "file-write": self._write_files,
            "file-read": self._read_files,
            "bash-light": self._run_light_shells,
            "network": self._run_network_requests,
            "bash-heavy": self._run_compiles,
            "mixed": self._run_mixed,
        }
        methods[operation](root, count)

    def _write_files(self, root: Path, count: int) -> None:
        for index in range(count):
            (root / f"write-{index:06d}.dat").write_bytes(self._payload)

    @staticmethod
    def _read_files(root: Path, count: int) -> None:
        checksum = 0
        for index in range(count):
            checksum ^= (root / f"read-{index:06d}.dat").read_bytes()[0]
        if checksum not in (0, ord("a")):
            raise RuntimeError("unexpected read checksum")

    @staticmethod
    def _run_light_shells(_root: Path, count: int) -> None:
        for _ in range(count):
            subprocess.run(["/bin/bash", "-c", ":"], check=True)

    def _run_network_requests(self, _root: Path, count: int) -> None:
        parsed = urlsplit(self.network_url)
        connection = http.client.HTTPConnection(
            parsed.hostname,
            parsed.port,
            timeout=10,
        )
        try:
            for _ in range(count):
                connection.request("GET", parsed.path or "/")
                response = connection.getresponse()
                response.read()
                if response.status != 204:
                    raise RuntimeError(
                        f"benchmark HTTP server returned {response.status}"
                    )
        finally:
            connection.close()

    def _run_compiles(self, root: Path, count: int) -> None:
        source = root / "compile.c"
        for index in range(count):
            output = root / f"compile-{index:06d}.o"
            subprocess.run(
                [
                    "/bin/bash",
                    "-c",
                    'exec "$1" -O2 -c "$2" -o "$3"',
                    "actrail-bench",
                    self.compiler,
                    str(source),
                    str(output),
                ],
                check=True,
            )

    def _run_mixed(self, root: Path, count: int) -> None:
        for index in range(count):
            path = root / f"write-{index:06d}.dat"
            path.write_bytes(self._payload)
            if (root / f"read-{index:06d}.dat").read_bytes() != self._payload:
                raise RuntimeError("mixed workload read mismatch")
            self._run_light_shells(root, 1)
            self._run_network_requests(root, 1)
            self._run_compiles(root, 1)

    def _create_read_files(self, root: Path, count: int) -> None:
        for index in range(count):
            (root / f"read-{index:06d}.dat").write_bytes(self._payload)

    @staticmethod
    def _create_compile_source(root: Path) -> None:
        (root / "compile.c").write_text(
            "#include <stdint.h>\n"
            "uint64_t actrail_mix(uint64_t value) {\n"
            "  for (uint64_t i = 0; i < 4096; ++i)\n"
            "    value = (value * 6364136223846793005ULL) + i;\n"
            "  return value;\n"
            "}\n",
            encoding="utf-8",
        )


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--operation", choices=OPERATION_NAMES, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--file-bytes", type=int, required=True)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--network-url", required=True)
    parser.add_argument("--elapsed-path", type=Path, required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = create_parser().parse_args(argv)
    suite = WorkloadSuite(
        args.root.parent,
        file_bytes=args.file_bytes,
        compiler=args.compiler,
        network_url=args.network_url,
    )
    started = time.perf_counter_ns()
    suite.run(args.operation, args.count, args.root)
    elapsed_ns = time.perf_counter_ns() - started
    args.elapsed_path.write_text(f"{elapsed_ns}\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
