"""x86_64 BoringSSL probe point detector."""

from __future__ import annotations

from pathlib import Path

from common import decode_hex_pattern, file_offset_to_virtual_address, find_all, symbol_map_text

from .patterns import (
    HANDSHAKE_PATTERN_HEX,
    KNOWN_SYMBOLS,
    READ_HANDSHAKE_DELTA,
    READ_PATTERN_HEX,
    WRITE_PATTERN_HEX,
    WRITE_READ_DELTA,
    WRITE_SEARCH_WINDOW,
)


ARCH = "x86_64"
LIBRARY = "boringssl"
RESOLVER = "bun-static-boringssl"
MAP_SYMBOLS = ("SSL_do_handshake", "SSL_read", "SSL_write")


def detect_patterns(binary: Path, target_build_id: str, match_limit: int) -> None:
    data = binary.read_bytes()
    patterns = {
        "SSL_do_handshake": decode_hex_pattern(HANDSHAKE_PATTERN_HEX),
        "SSL_read": decode_hex_pattern(READ_PATTERN_HEX),
        "SSL_write": decode_hex_pattern(WRITE_PATTERN_HEX),
    }
    matches = {symbol: find_all(data, pattern) for symbol, pattern in patterns.items()}
    print("--- x86_64 known pattern matches ---")
    for symbol, offsets in matches.items():
        print(f"symbol={symbol}")
        print(f"library={LIBRARY}")
        print(f"resolver={RESOLVER}")
        print(f"pattern_length=0x{len(patterns[symbol]):x}")
        print(f"match_count={len(offsets)}")
        for file_offset in offsets[:match_limit]:
            address = file_offset_to_virtual_address(binary, file_offset)
            print(f"match file_offset=0x{file_offset:x} virtual_address=0x{address:x}")
        print("---")

    read_offset = require_single(matches["SSL_read"], "SSL_read")
    handshake_offset = resolve_handshake(data, matches["SSL_do_handshake"], read_offset)
    write_offset = resolve_write(data, matches["SSL_write"], read_offset)
    offsets = {
        "SSL_do_handshake": handshake_offset,
        "SSL_read": read_offset,
        "SSL_write": write_offset,
    }
    print("--- detected offsets ---")
    symbols = {}
    for symbol, file_offset in offsets.items():
        address = file_offset_to_virtual_address(binary, file_offset)
        symbols[symbol] = address
        print(f"{symbol} file_offset=0x{file_offset:x} virtual_address=0x{address:x}")
    print("--- candidate symbol maps ---")
    print(
        symbol_map_text(
            resolver=RESOLVER,
            library=LIBRARY,
            arch=ARCH,
            target_build_id=target_build_id,
            symbols=symbols,
        ),
        end="",
    )


def resolve_handshake(data: bytes, handshake_offsets: list[int], read_offset: int) -> int:
    pattern = decode_hex_pattern(HANDSHAKE_PATTERN_HEX)
    expected = read_offset - READ_HANDSHAKE_DELTA
    if expected >= 0 and data[expected : expected + len(pattern)] == pattern:
        return expected
    return require_single(handshake_offsets, "SSL_do_handshake")


def resolve_write(data: bytes, write_offsets: list[int], read_offset: int) -> int:
    pattern = decode_hex_pattern(WRITE_PATTERN_HEX)
    expected = read_offset + WRITE_READ_DELTA
    if data[expected : expected + len(pattern)] == pattern:
        return expected
    search_end = min(len(data), read_offset + WRITE_SEARCH_WINDOW)
    nearby = [read_offset + offset for offset in find_all(data[read_offset:search_end], pattern)]
    if len(nearby) == 1:
        return nearby[0]
    if len(write_offsets) == 1:
        return write_offsets[0]
    raise RuntimeError(f"BoringSSL SSL_write nearby pattern match count={len(nearby)}")


def require_single(offsets: list[int], symbol: str) -> int:
    if len(offsets) != 1:
        raise RuntimeError(f"BoringSSL {symbol} pattern match count={len(offsets)}")
    return offsets[0]
