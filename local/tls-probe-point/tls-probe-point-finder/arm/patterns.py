"""ARM64 BoringSSL probe point signatures."""

KNOWN_SYMBOLS = ("SSL_write", "SSL_write_ex", "SSL_read", "SSL_read_ex")

# User-approved exception to the no-magic-number rule: these documented,
# runtime-validated ARM64 BoringSSL entry signatures and deltas are detector
# anchors, not heuristic knobs. Validation samples are recorded in local docs.
READ_WRAPPER_PATTERN_HEX = (
    "fd 7b bd a9 f5 0b 00 f9 f4 4f 02 a9 fd 03 00 91 "
    "08 4c 40 f9 a8 01 00 b4"
)
READ_INTERNAL_PATTERN_HEX = (
    "ff 03 02 d1 fd 7b 04 a9 f8 5f 05 a9 f6 57 06 a9 "
    "f4 4f 07 a9 fd 03 01 91 08 18 40 f9 f3 03 00 aa"
)
WRITE_PATTERN_HEX = (
    "ff 03 01 d1 fd 7b 01 a9 f6 57 02 a9 f4 4f 03 a9 "
    "fd 43 00 91 08 18 40 f9 f5 03 02 2a f4 03 01 aa"
)
WRITE_READ_WRAPPER_DELTA = 0x3C0
WRITE_READ_INTERNAL_DELTA = 0x2C0

KNOWN_PATTERNS: tuple[dict[str, str], ...] = (
    {
        "id": "arm64-boringssl-ssl-read-wrapper-24",
        "symbol": "SSL_read",
        "bytes": READ_WRAPPER_PATTERN_HEX,
    },
    {
        "id": "arm64-boringssl-ssl-read-internal-32",
        "symbol": "SSL_read_internal",
        "bytes": READ_INTERNAL_PATTERN_HEX,
    },
    {
        "id": "arm64-boringssl-ssl-write-entry-32",
        "symbol": "SSL_write",
        "bytes": WRITE_PATTERN_HEX,
    },
)
