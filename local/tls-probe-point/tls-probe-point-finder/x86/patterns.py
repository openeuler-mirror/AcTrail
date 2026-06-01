"""x86_64 BoringSSL probe point signatures."""

KNOWN_SYMBOLS = ("SSL_do_handshake", "SSL_read", "SSL_write", "SSL_write_ex")

# These are the documented x86_64 static-BoringSSL signatures used by the
# existing AcTrail agent examples. They identify related BoringSSL entry points
# by a unique SSL_read match plus stable read/handshake and write/read deltas.
HANDSHAKE_PATTERN_HEX = (
    "55 48 89 e5 41 57 41 56 41 55 41 54 53 48 83 ec 28 "
    "49 89 fc 48 8b 47 30"
)
READ_PATTERN_HEX = "55 48 89 e5 41 57 41 56 53 50 48 83 bf 98 00 00 00 00 74"
WRITE_PATTERN_HEX = (
    "55 48 89 e5 41 57 41 56 41 55 41 54 53 48 83 ec 18 "
    "41 89 d7 49 89 f6 48 89 fb"
)
READ_HANDSHAKE_DELTA = 0x6F0
WRITE_READ_DELTA = 0xCA0
WRITE_SEARCH_WINDOW = 0x10000
