mod brotli;
mod gzip;
mod identity;

pub(super) use brotli::BrotliDecoder;
pub(super) use gzip::GzipDecoder;
pub(super) use identity::IdentityDecoder;
