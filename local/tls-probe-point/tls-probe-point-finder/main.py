#!/usr/bin/env python3
"""CLI for detecting TLS plaintext probe points."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from arm import detector as arm_detector
from arm import uprobes as arm_uprobes
from common import (
    build_id,
    elf_arch,
    file_offset_to_virtual_address,
    find_all,
    require_arch,
    resolve_entry_elf,
    symbols_by_name,
    unique_exported_symbols,
    virtual_address_to_file_offset,
)
from x86 import detector as x86_detector
from x86 import uprobes as x86_uprobes


DETECTORS = {
    arm_detector.ARCH: arm_detector,
    x86_detector.ARCH: x86_detector,
}
UPROBES = {
    arm_detector.ARCH: arm_uprobes,
    x86_detector.ARCH: x86_uprobes,
}


def main() -> int:
    args = parse_args()
    if args.command == "detect":
        detect(args)
        return 0
    if args.command == "pattern":
        print_pattern(args)
        return 0
    if args.command == "trace":
        trace(args)
        return 0
    raise RuntimeError(f"unknown command {args.command}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    detect_parser = subparsers.add_parser("detect")
    add_binary_arch_args(detect_parser)
    detect_parser.add_argument("--symbol", action="append")
    detect_parser.add_argument("--match-limit", type=int, default=8)

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


def detect(args: argparse.Namespace) -> None:
    binary = resolve_entry_elf(args.binary)
    arch = selected_arch(binary, args.arch)
    detector = DETECTORS[arch]
    target_build_id = build_id(binary)
    print(f"binary={binary}")
    print(f"architecture={arch}")
    print(f"build_id={target_build_id}")
    names = tuple(dict.fromkeys((*detector.KNOWN_SYMBOLS, *(args.symbol or []))))
    print_exported_symbols(binary, names)
    exported = unique_exported_symbols(binary, detector.MAP_SYMBOLS)
    if exported:
        print("--- exported symbol maps ---")
        print(
            detector_symbol_map(detector, target_build_id, exported),
            end="",
        )
        return
    detector.detect_patterns(binary, target_build_id, args.match_limit)


def print_exported_symbols(binary: Path, names: tuple[str, ...]) -> None:
    print("--- exported symbols ---")
    symbols = symbols_by_name(binary, names)
    for name in names:
        matches = symbols.get(name, [])
        if not matches:
            print(f"{name}=not_found")
            continue
        for match in matches:
            print(
                f"{name}=0x{match['value']:x} size=0x{match['size']:x} "
                f"bind={match['bind']} ndx={match['ndx']} raw={match['raw_name']}"
            )


def detector_symbol_map(detector, target_build_id: str, symbols: dict[str, int]) -> str:
    from common import symbol_map_text

    return symbol_map_text(
        resolver=detector.RESOLVER,
        library=detector.LIBRARY,
        arch=detector.ARCH,
        target_build_id=target_build_id,
        symbols=symbols,
    )


def print_pattern(args: argparse.Namespace) -> None:
    binary = resolve_entry_elf(args.binary)
    selected_arch(binary, args.arch)
    address = parse_int(args.address)
    file_offset = virtual_address_to_file_offset(binary, address)
    data = binary.read_bytes()
    end = file_offset + args.length
    if end > len(data):
        raise RuntimeError(f"pattern range exceeds file size at 0x{file_offset:x}")
    pattern = data[file_offset:end]
    if not pattern:
        raise RuntimeError("pattern length must be non-zero")
    matches = find_all(data, pattern)
    print(f"address=0x{address:x}")
    print(f"file_offset=0x{file_offset:x}")
    print(f"length=0x{args.length:x}")
    print(f"match_count={len(matches)}")
    print("pattern_hex=" + " ".join(f"{byte:02x}" for byte in pattern))
    for match in matches[: args.match_limit]:
        address = file_offset_to_virtual_address(binary, match)
        print(f"match file_offset=0x{match:x} virtual_address=0x{address:x}")


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
