mod brotli;
mod gzip;
mod identity;
mod zstd;

pub(super) use brotli::BrotliDecoder;
pub(super) use gzip::GzipDecoder;
pub(super) use identity::IdentityDecoder;
pub(super) use zstd::ZstdDecoder;
