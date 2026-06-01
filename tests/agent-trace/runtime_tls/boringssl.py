#!/usr/bin/env python3
"""Discover static BoringSSL SSL_write offsets for executable TLS tests."""

from __future__ import annotations

import platform
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class DetectionResult:
    build_id: str
    ssl_read_file_offset: int
    ssl_write_file_offset: int
    ssl_read_virtual_address: int
    ssl_write_virtual_address: int
    method: str


def prepare_bun_static_boringssl_map(
    binary: Path,
    configured_map: Path,
    settings: dict[str, str],
) -> tuple[Path, str]:
    build_id = binary_build_id(binary)
    if symbol_map_matches(configured_map, build_id, ("SSL_read", "SSL_write")):
        return configured_map, f"configured map matches build_id={build_id}"

    detection = detect_boringssl_offsets(binary, settings)
    generated_map = Path(required(settings, "generated_symbol_map_path"))
    write_symbol_map(generated_map, detection)
    return generated_map, (
        f"generated map from {detection.method} for build_id={detection.build_id} "
        f"SSL_read=0x{detection.ssl_read_virtual_address:x} "
        f"SSL_write=0x{detection.ssl_write_virtual_address:x}"
    )


def detect_boringssl_offsets(binary: Path, settings: dict[str, str]) -> DetectionResult:
    build_id = binary_build_id(binary)
    symbol_addresses = ssl_symbol_addresses(binary, ("SSL_read", "SSL_write"))
    if symbol_addresses is not None:
        read_address = symbol_addresses["SSL_read"]
        write_address = symbol_addresses["SSL_write"]
        return DetectionResult(
            build_id=build_id,
            ssl_read_file_offset=virtual_address_to_file_offset(binary, read_address),
            ssl_write_file_offset=virtual_address_to_file_offset(binary, write_address),
            ssl_read_virtual_address=read_address,
            ssl_write_virtual_address=write_address,
            method="ELF symbol table",
        )
    return detect_boringssl_offsets_by_pattern(binary, settings, build_id)


def detect_boringssl_offsets_by_pattern(
    binary: Path,
    settings: dict[str, str],
    build_id: str,
) -> DetectionResult:
    configured_arch = required(settings, "boringssl_pattern_arch")
    current_arch = platform.machine()
    if configured_arch != current_arch:
        raise RuntimeError(
            f"BoringSSL byte patterns are for arch={configured_arch}, current arch={current_arch}; "
            "provide a matching symbol map or arch-specific patterns"
        )

    data = binary.read_bytes()
    read_pattern = bytes.fromhex(required(settings, "boringssl_read_pattern_hex"))
    write_pattern = bytes.fromhex(required(settings, "boringssl_write_pattern_hex"))
    handshake_pattern = bytes.fromhex(required(settings, "boringssl_handshake_pattern_hex"))
    read_handshake_delta = parse_config_int(required(settings, "boringssl_read_handshake_delta"))
    write_read_delta = parse_config_int(required(settings, "boringssl_write_read_delta"))
    write_search_window = parse_config_int(required(settings, "boringssl_write_search_window"))

    read_offsets = find_all(data, read_pattern)
    if len(read_offsets) != 1:
        raise RuntimeError(f"BoringSSL SSL_read pattern match count={len(read_offsets)}")
    read_offset = read_offsets[0]
    expected_handshake = read_offset - read_handshake_delta
    if expected_handshake < 0 or data[expected_handshake : expected_handshake + len(handshake_pattern)] != handshake_pattern:
        handshake_offsets = find_all(data, handshake_pattern)
        if len(handshake_offsets) != 1:
            raise RuntimeError(f"BoringSSL SSL_do_handshake pattern match count={len(handshake_offsets)}")

    expected_write = read_offset + write_read_delta
    if data[expected_write : expected_write + len(write_pattern)] == write_pattern:
        write_offset = expected_write
    else:
        search_end = min(len(data), read_offset + write_search_window)
        matches = [offset for offset in find_all(data[read_offset:search_end], write_pattern)]
        if len(matches) != 1:
            raise RuntimeError(f"BoringSSL SSL_write nearby pattern match count={len(matches)}")
        write_offset = read_offset + matches[0]

    return DetectionResult(
        build_id=build_id,
        ssl_read_file_offset=read_offset,
        ssl_write_file_offset=write_offset,
        ssl_read_virtual_address=file_offset_to_virtual_address(binary, read_offset),
        ssl_write_virtual_address=file_offset_to_virtual_address(binary, write_offset),
        method="configured byte patterns",
    )


def ssl_symbol_addresses(binary: Path, symbols: tuple[str, ...]) -> dict[str, int] | None:
    output = run_checked(["readelf", "-Ws", str(binary)])
    matches: dict[str, set[int]] = {symbol: set() for symbol in symbols}
    for line in output.splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0].endswith(":"):
            continue
        symbol_type = parts[3]
        section_index = parts[6]
        symbol_name = parts[7].split("@", 1)[0]
        if symbol_type != "FUNC" or section_index == "UND" or symbol_name not in matches:
            continue
        matches[symbol_name].add(int(parts[1], 16))
    if any(not addresses for addresses in matches.values()):
        return None
    resolved = {}
    for symbol, addresses in matches.items():
        if len(addresses) != 1:
            formatted = ", ".join(f"0x{address:x}" for address in sorted(addresses))
            raise RuntimeError(f"ELF symbol table has multiple {symbol} addresses: {formatted}")
        resolved[symbol] = next(iter(addresses))
    return resolved


def write_symbol_map(path: Path, detection: DetectionResult) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Generated by tests/agent-trace/runtime_tls/boringssl.py",
        "resolver = bun-static-boringssl",
        "library = boringssl",
        f"arch = {platform.machine()}",
        f"build_id = {detection.build_id}",
        f"symbol = SSL_read|0x{detection.ssl_read_virtual_address:x}",
        f"symbol = SSL_write|0x{detection.ssl_write_virtual_address:x}",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def symbol_map_matches(path: Path, build_id: str, symbols: tuple[str, ...]) -> bool:
    if not path.exists():
        return False
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^build_id\s*=\s*([0-9a-fA-F]+)\s*$", text, re.MULTILINE)
    if match is None or match.group(1).lower() != build_id:
        return False
    return all(
        re.search(rf"^symbol\s*=\s*{re.escape(symbol)}\|0x[0-9a-fA-F]+\s*$", text, re.MULTILINE)
        for symbol in symbols
    )


def binary_build_id(binary: Path) -> str:
    output = run_checked(["readelf", "-n", str(binary)])
    match = re.search(r"Build ID:\s*([0-9a-fA-F]+)", output)
    if not match:
        raise RuntimeError(f"{binary} has no GNU build-id")
    return match.group(1).lower()


def file_offset_to_virtual_address(binary: Path, file_offset: int) -> int:
    for segment_offset, virtual_address, file_size in load_segments(binary):
        if segment_offset <= file_offset < segment_offset + file_size:
            return virtual_address + file_offset - segment_offset
    raise RuntimeError(f"file offset 0x{file_offset:x} is not inside a LOAD segment")


def virtual_address_to_file_offset(binary: Path, address: int) -> int:
    for segment_offset, virtual_address, file_size in load_segments(binary):
        if virtual_address <= address < virtual_address + file_size:
            return segment_offset + address - virtual_address
    raise RuntimeError(f"virtual address 0x{address:x} is not inside a LOAD segment")


def load_segments(binary: Path) -> list[tuple[int, int, int]]:
    output = run_checked(["readelf", "-lW", str(binary)])
    segments: list[tuple[int, int, int]] = []
    for line in output.splitlines():
        parts = line.split()
        if not parts or parts[0] != "LOAD" or len(parts) < 6:
            continue
        segments.append((int(parts[1], 16), int(parts[2], 16), int(parts[4], 16)))
    if not segments:
        raise RuntimeError(f"{binary} has no LOAD segments")
    return segments


def find_all(data: bytes, pattern: bytes) -> list[int]:
    if not pattern:
        raise RuntimeError("BoringSSL pattern must not be empty")
    offsets = []
    start = 0
    while True:
        offset = data.find(pattern, start)
        if offset < 0:
            return offsets
        offsets.append(offset)
        start = offset + 1


def parse_config_int(value: str) -> int:
    return int(value, 0)


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing BoringSSL workload config key {key}")
    return value


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout
