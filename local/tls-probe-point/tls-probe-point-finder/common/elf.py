"""ELF metadata helpers."""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class LoadSegment:
    file_offset: int
    virtual_address: int
    file_size: int


def machine_name(binary: Path) -> str:
    output = run_checked(["readelf", "-h", str(binary)])
    for line in output.splitlines():
        if "Machine:" in line:
            return line.split("Machine:", maxsplit=1)[1].strip()
    raise RuntimeError(f"cannot read ELF machine from {binary}")


def elf_arch(binary: Path) -> str:
    machine = machine_name(binary)
    if machine == "AArch64":
        return "aarch64"
    if "X86-64" in machine:
        return "x86_64"
    raise RuntimeError(f"unsupported ELF machine: {machine}")


def require_arch(binary: Path, expected: str) -> None:
    actual = elf_arch(binary)
    if actual != expected:
        raise RuntimeError(f"{binary} is {actual}, not {expected}")


def build_id(binary: Path) -> str:
    output = run_checked(["readelf", "-n", str(binary)])
    for line in output.splitlines():
        marker = "Build ID:"
        if marker in line:
            return line.split(marker, maxsplit=1)[1].strip()
    raise RuntimeError(f"{binary} has no GNU build ID")


def symbols_by_name(binary: Path, names: tuple[str, ...]) -> dict[str, list[dict[str, Any]]]:
    wanted = set(names)
    found: dict[str, list[dict[str, Any]]] = {name: [] for name in names}
    current_table = "unknown"
    output = run_checked(["readelf", "-Ws", str(binary)])
    for line in output.splitlines():
        if line.startswith("Symbol table '"):
            current_table = line.split("'", maxsplit=2)[1]
            continue
        parts = line.split()
        if len(parts) < len(("Num", "Value", "Size", "Type", "Bind", "Vis", "Ndx", "Name")):
            continue
        if not parts[0].endswith(":") or parts[3] != "FUNC":
            continue
        raw_name = parts[7]
        name = raw_name.split("@", maxsplit=1)[0]
        if name not in wanted:
            continue
        value = int(parts[1], 16)
        size = int(parts[2], 10)
        found[name].append(
            {
                "value": value,
                "size": size,
                "table": current_table,
                "bind": parts[4],
                "ndx": parts[6],
                "raw_name": raw_name,
            }
        )
    return found


def unique_exported_symbols(binary: Path, names: tuple[str, ...]) -> dict[str, int]:
    symbols = symbols_by_name(binary, names)
    resolved: dict[str, int] = {}
    for name, matches in symbols.items():
        addresses = {int(match["value"]) for match in matches if match["ndx"] != "UND"}
        if not addresses:
            continue
        if len(addresses) != 1:
            formatted = ", ".join(format_symbol_location(match) for match in matches)
            raise RuntimeError(f"ELF symbol table has multiple {name} addresses: {formatted}")
        resolved[name] = next(iter(addresses))
    return resolved


def format_symbol_location(match: dict[str, Any]) -> str:
    return f"0x{match['value']:x}@{match['table']}"


def load_segments(binary: Path) -> list[LoadSegment]:
    output = run_checked(["readelf", "-lW", str(binary)])
    segments: list[LoadSegment] = []
    for line in output.splitlines():
        parts = line.split()
        if parts and parts[0] == "LOAD":
            segments.append(
                LoadSegment(
                    file_offset=int(parts[1], 16),
                    virtual_address=int(parts[2], 16),
                    file_size=int(parts[4], 16),
                )
            )
    if not segments:
        raise RuntimeError(f"{binary} has no LOAD segments")
    return segments


def virtual_address_to_file_offset(binary: Path, address: int) -> int:
    for segment in load_segments(binary):
        start = segment.virtual_address
        end = segment.virtual_address + segment.file_size
        if start <= address < end:
            return segment.file_offset + address - segment.virtual_address
    raise RuntimeError(f"virtual address 0x{address:x} is not inside a LOAD segment")


def file_offset_to_virtual_address(binary: Path, file_offset: int) -> int:
    for segment in load_segments(binary):
        start = segment.file_offset
        end = segment.file_offset + segment.file_size
        if start <= file_offset < end:
            return segment.virtual_address + file_offset - segment.file_offset
    raise RuntimeError(f"file offset 0x{file_offset:x} is not inside a LOAD segment")


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout
