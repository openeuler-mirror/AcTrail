//! x86_64 static BoringSSL probe resolution.

use crate::loader::LoaderError;

use super::{StaticBoringSslOffsets, find_all, matches_at, require_single};

// Documented and runtime-validated in local/tls-probe-point.
const SSL_HANDSHAKE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x28, 0x49, 0x89, 0xfc, 0x48, 0x8b, 0x47, 0x30,
];
const SSL_READ_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x53, 0x50, 0x48, 0x83, 0xbf, 0x98, 0x00, 0x00,
    0x00, 0x00, 0x74,
];
const SSL_WRITE_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xec,
    0x18, 0x41, 0x89, 0xd7, 0x49, 0x89, 0xf6, 0x48, 0x89, 0xfb,
];
const READ_HANDSHAKE_DELTA: usize = 0x6f0;
const WRITE_READ_DELTA: usize = 0xca0;
const WRITE_SEARCH_WINDOW: usize = 0x10000;

pub(super) fn detect(data: &[u8]) -> Result<StaticBoringSslOffsets, LoaderError> {
    let read = require_single(find_all(data, SSL_READ_PATTERN), "SSL_read")?;
    resolve_handshake(data, read)?;
    let write = resolve_write(data, read)?;
    Ok(StaticBoringSslOffsets {
        ssl_read: read,
        ssl_write: write,
    })
}

fn resolve_handshake(data: &[u8], read: usize) -> Result<usize, LoaderError> {
    let Some(expected) = read.checked_sub(READ_HANDSHAKE_DELTA) else {
        return require_single(find_all(data, SSL_HANDSHAKE_PATTERN), "SSL_do_handshake");
    };
    if matches_at(data, expected, SSL_HANDSHAKE_PATTERN) {
        Ok(expected)
    } else {
        require_single(find_all(data, SSL_HANDSHAKE_PATTERN), "SSL_do_handshake")
    }
}

fn resolve_write(data: &[u8], read: usize) -> Result<usize, LoaderError> {
    let expected = read + WRITE_READ_DELTA;
    if matches_at(data, expected, SSL_WRITE_PATTERN) {
        return Ok(expected);
    }
    let search_end = data.len().min(read + WRITE_SEARCH_WINDOW);
    let nearby = find_all(&data[read..search_end], SSL_WRITE_PATTERN)
        .into_iter()
        .map(|offset| read + offset)
        .collect::<Vec<_>>();
    if nearby.len() == 1 {
        return Ok(nearby[0]);
    }
    require_single(find_all(data, SSL_WRITE_PATTERN), "SSL_write")
}
