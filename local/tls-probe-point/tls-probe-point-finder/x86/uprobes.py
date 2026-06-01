"""x86_64 runtime uprobe verification for TLS probe point candidates."""

from __future__ import annotations

import re
from pathlib import Path

from common import (
    command_after_delimiter,
    print_target_result,
    require_arch,
    resolve_entry_elf,
    run_target_for_duration,
    validate_trace_group,
    virtual_address_to_file_offset,
    write_text,
)


def trace_addresses(args) -> None:
    binary = resolve_entry_elf(args.binary)
    require_arch(binary, "x86_64")
    validate_trace_group(args.group)
    target_command = command_after_delimiter(args.target_command)
    addresses = [int(raw, 0) for raw in args.address]
    tracefs = args.tracefs
    if not (tracefs / "uprobe_events").exists():
        raise RuntimeError(f"{tracefs} does not look like tracefs")

    cleanup_group(tracefs, args.group)
    write_text(tracefs / "trace", "\n")
    try:
        for index, address in enumerate(addresses):
            file_offset = virtual_address_to_file_offset(binary, address)
            event_line = f"p:{args.group}/p{index} {binary}:0x{file_offset:x} rdi=%di rsi=%si rdx=%dx\n"
            write_text(tracefs / "uprobe_events", event_line)
            print(f"event=p{index} address=0x{address:x} file_offset=0x{file_offset:x}")
        if args.event_filter:
            write_text(tracefs / "events" / args.group / "filter", args.event_filter + "\n")
        write_text(tracefs / "events" / args.group / "enable", "1\n")
        result = run_target_for_duration(target_command, args.target_timeout_seconds)
        write_text(tracefs / "events" / args.group / "enable", "0\n")
        trace = (tracefs / "trace").read_text(encoding="utf-8", errors="replace")
    finally:
        cleanup_group(tracefs, args.group)

    print_target_result(result)
    print("--- hit summary ---")
    print_hit_summary(trace)
    print("--- trace samples ---")
    print_trace_samples(trace, args.sample_limit)


def cleanup_group(tracefs: Path, group: str) -> None:
    group_dir = tracefs / "events" / group
    if group_dir.exists() and (group_dir / "enable").exists():
        write_text(group_dir / "enable", "0\n")
    events_file = tracefs / "uprobe_events"
    if not events_file.exists():
        return
    raw = events_file.read_text(encoding="utf-8", errors="replace")
    names = []
    for line in raw.splitlines():
        match = re.match(r"^[pr]:([^/]+)/([^\s]+)", line)
        if match and match.group(1) == group:
            names.append(match.group(2))
    for name in names:
        try:
            write_text(events_file, f"-:{group}/{name}\n")
        except FileNotFoundError:
            pass


def print_hit_summary(trace: str) -> None:
    pattern = re.compile(r":\s+(p\d+):.*?\brdx=(0x[0-9a-fA-F]+|0x)")
    hits: dict[str, list[int | None]] = {}
    for line in trace.splitlines():
        match = pattern.search(line)
        if match:
            raw = match.group(2)
            hits.setdefault(match.group(1), []).append(None if raw == "0x" else int(raw, 16))
    for event, values in sorted(hits.items(), key=lambda item: (-len(item[1]), item[0])):
        numeric = [value for value in values if value is not None]
        max_rdx = "none" if not numeric else f"0x{max(numeric):x}"
        print(f"{event} count={len(values)} max_rdx={max_rdx}")


def print_trace_samples(trace: str, limit: int) -> None:
    emitted = 0
    for line in trace.splitlines():
        if ": p" in line:
            print(line)
            emitted += 1
            if emitted >= limit:
                return
