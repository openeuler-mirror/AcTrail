#!/usr/bin/env python3
"""Legacy CLI for manual TLS tracefs experiments."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from arm import uprobes as arm_uprobes
from common import elf_arch, require_arch, resolve_entry_elf
from x86 import uprobes as x86_uprobes


UPROBES = {
    "aarch64": arm_uprobes,
    "x86_64": x86_uprobes,
}

RUST_TOOL = "target/debug/tls-probe-point-finder"


def main() -> int:
    args = parse_args()
    if args.command == "detect":
        retired("detect")
    if args.command == "pattern":
        retired("pattern")
    if args.command == "trace":
        trace(args)
        return 0
    raise RuntimeError(f"unknown command {args.command}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    detect_parser = subparsers.add_parser("detect")
    add_binary_arch_args(detect_parser)
    detect_parser.add_argument("--provider", choices=("auto", "openssl", "boringssl", "rustls"), default="auto")
    detect_parser.add_argument("--source", choices=("auto", "executable", "shared-library"), default="auto")
    detect_parser.add_argument("--symbol", action="append")
    detect_parser.add_argument("--match-limit", type=int, default=8)
    detect_parser.add_argument("--library", action="append", type=Path)
    detect_parser.add_argument("--library-search-dir", action="append", type=Path)

    pattern_parser = subparsers.add_parser("pattern")
    add_binary_arch_args(pattern_parser)
    pattern_parser.add_argument("--address", required=True)
    pattern_parser.add_argument("--length", type=parse_int, required=True)
    pattern_parser.add_argument("--match-limit", type=int, default=8)

    trace_parser = subparsers.add_parser("trace")
    add_binary_arch_args(trace_parser)
    trace_parser.add_argument("--address", action="append", required=True)
    trace_parser.add_argument("--tracefs", type=Path, required=True)
    trace_parser.add_argument("--group", required=True)
    trace_parser.add_argument("--target-timeout-seconds", type=float, required=True)
    trace_parser.add_argument("--sample-limit", type=int, required=True)
    trace_parser.add_argument("--event-filter")
    trace_parser.add_argument("--fetch-x1-string", action="store_true")
    trace_parser.add_argument("target_command", nargs=argparse.REMAINDER)
    return parser.parse_args()


def add_binary_arch_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("binary", type=Path)
    parser.add_argument("--arch", choices=("auto", "aarch64", "x86_64"), default="auto")


def retired(command: str) -> None:
    raise RuntimeError(
        f"python {command} is retired; build and run {RUST_TOOL} {command} instead"
    )


def trace(args: argparse.Namespace) -> None:
    binary = resolve_entry_elf(args.binary)
    arch = selected_arch(binary, args.arch)
    UPROBES[arch].trace_addresses(args)


def selected_arch(binary: Path, requested: str) -> str:
    actual = elf_arch(binary)
    if requested == "auto":
        return actual
    require_arch(binary, requested)
    return requested


def parse_int(value: str) -> int:
    return int(value, 0)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
