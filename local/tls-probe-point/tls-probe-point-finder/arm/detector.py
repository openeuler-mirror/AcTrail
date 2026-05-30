"""ARM64 BoringSSL probe point detector."""

from __future__ import annotations

from pathlib import Path

from common import (
    decode_hex_pattern,
    file_offset_to_virtual_address,
    find_all,
    symbol_map_text,
)

from .patterns import (
    KNOWN_PATTERNS,
    KNOWN_SYMBOLS,
    WRITE_READ_INTERNAL_DELTA,
    WRITE_READ_WRAPPER_DELTA,
)


ARCH = "aarch64"
LIBRARY = "boringssl"
RESOLVER = "bun-static-boringssl"
MAP_SYMBOLS = ("SSL_read", "SSL_write")


def detect_patterns(binary: Path, target_build_id: str, match_limit: int) -> None:
    data = binary.read_bytes()
    pattern_bytes = {
        pattern["symbol"]: decode_hex_pattern(pattern["bytes"])
        for pattern in KNOWN_PATTERNS
    }
    matches = {symbol: find_all(data, pattern) for symbol, pattern in pattern_bytes.items()}
    print("--- aarch64 known pattern matches ---")
    for pattern in KNOWN_PATTERNS:
        print(f"pattern_id={pattern['id']}")
        print(f"symbol={pattern['symbol']}")
        print(f"library={LIBRARY}")
        print(f"resolver={RESOLVER}")
        print(f"pattern_length=0x{len(pattern_bytes[pattern['symbol']]):x}")
        print(f"match_count={len(matches[pattern['symbol']])}")
        for file_offset in matches[pattern["symbol"]][:match_limit]:
            address = file_offset_to_virtual_address(binary, file_offset)
            print(f"match file_offset=0x{file_offset:x} virtual_address=0x{address:x}")
        print("---")

    write_offset = require_single(matches["SSL_write"], "SSL_write")
    read_offset = require_related(
        matches["SSL_read"],
        write_offset,
        WRITE_READ_WRAPPER_DELTA,
        "SSL_read",
    )
    read_internal_offset = require_related(
        matches["SSL_read_internal"],
        write_offset,
        WRITE_READ_INTERNAL_DELTA,
        "SSL_read_internal",
    )
    offsets = {
        "SSL_read": read_offset,
        "SSL_read_internal": read_internal_offset,
        "SSL_write": write_offset,
    }
    print("--- detected offsets ---")
    symbols = {}
    for symbol, file_offset in offsets.items():
        address = file_offset_to_virtual_address(binary, file_offset)
        print(f"{symbol} file_offset=0x{file_offset:x} virtual_address=0x{address:x}")
        if symbol in MAP_SYMBOLS:
            symbols[symbol] = address
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


def require_related(
    offsets: list[int],
    write_offset: int,
    delta: int,
    symbol: str,
) -> int:
    offset = require_single(offsets, symbol)
    expected = write_offset - delta
    if offset != expected:
        raise RuntimeError(f"BoringSSL {symbol} is not at SSL_write-0x{delta:x}")
    return offset


def require_single(offsets: list[int], symbol: str) -> int:
    if len(offsets) != 1:
        raise RuntimeError(f"BoringSSL {symbol} pattern match count={len(offsets)}")
    return offsets[0]
