"""Pattern and symbol-map helpers."""

from __future__ import annotations


def find_all(data: bytes, pattern: bytes) -> list[int]:
    if not pattern:
        raise RuntimeError("pattern must not be empty")
    offsets = []
    start = 0
    while True:
        offset = data.find(pattern, start)
        if offset < 0:
            return offsets
        offsets.append(offset)
        start = offset + 1


def decode_hex_pattern(text: str) -> bytes:
    compact = "".join(text.split())
    if not compact or len(compact) % 2:
        raise RuntimeError("pattern bytes must be even-length hexadecimal text")
    try:
        return bytes.fromhex(compact)
    except ValueError as error:
        raise RuntimeError("pattern bytes must be hexadecimal text") from error


def symbol_map_text(
    *,
    resolver: str,
    library: str,
    arch: str,
    target_build_id: str,
    symbols: dict[str, int],
) -> str:
    lines = [
        f"resolver = {resolver}",
        f"library = {library}",
        f"arch = {arch}",
        f"build_id = {target_build_id}",
    ]
    lines.extend(f"symbol = {symbol}|0x{address:x}" for symbol, address in symbols.items())
    return "\n".join(lines) + "\n"
