//! ARM64 static BoringSSL probe resolution.

use crate::loader::LoaderError;

use super::{StaticBoringSslOffsets, find_all, matches_at, require_single};

// Documented and runtime-validated in local/tls-probe-point.
const SSL_READ_PATTERN: &[u8] = &[
    0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91,
    0x08, 0x4c, 0x40, 0xf9, 0xa8, 0x01, 0x00, 0xb4,
];
const SSL_READ_INTERNAL_PATTERN: &[u8] = &[
    0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf8, 0x5f, 0x05, 0xa9, 0xf6, 0x57, 0x06, 0xa9,
    0xf4, 0x4f, 0x07, 0xa9, 0xfd, 0x03, 0x01, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf3, 0x03, 0x00, 0xaa,
];
const SSL_WRITE_PATTERN: &[u8] = &[
    0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf6, 0x57, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9,
    0xfd, 0x43, 0x00, 0x91, 0x08, 0x18, 0x40, 0xf9, 0xf5, 0x03, 0x02, 0x2a, 0xf4, 0x03, 0x01, 0xaa,
];
const WRITE_READ_DELTA: usize = 0x3c0;
const WRITE_READ_INTERNAL_DELTA: usize = 0x2c0;

pub(super) fn detect(data: &[u8]) -> Result<StaticBoringSslOffsets, LoaderError> {
    let write = require_single(find_all(data, SSL_WRITE_PATTERN), "SSL_write")?;
    let read = require_related(data, write, SSL_READ_PATTERN, WRITE_READ_DELTA, "SSL_read")?;
    require_related(
        data,
        write,
        SSL_READ_INTERNAL_PATTERN,
        WRITE_READ_INTERNAL_DELTA,
        "SSL_read_internal",
    )?;
    Ok(StaticBoringSslOffsets {
        ssl_read: read,
        ssl_write: write,
    })
}

fn require_related(
    data: &[u8],
    write: usize,
    pattern: &[u8],
    delta: usize,
    symbol: &str,
) -> Result<usize, LoaderError> {
    let offset = require_single(find_all(data, pattern), symbol)?;
    let expected = write.checked_sub(delta).ok_or_else(|| {
        LoaderError::new(
            "payload_tls_resolver",
            format!("BoringSSL {symbol} offset underflows SSL_write delta"),
        )
    })?;
    if offset == expected && matches_at(data, expected, pattern) {
        Ok(offset)
    } else {
        Err(LoaderError::new(
            "payload_tls_resolver",
            format!("BoringSSL {symbol} is not at SSL_write-0x{delta:x}"),
        ))
    }
}
