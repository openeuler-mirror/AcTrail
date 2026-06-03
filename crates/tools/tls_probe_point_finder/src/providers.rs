//! TLS provider detector modules.

#[path = "providers/boringssl.rs"]
pub(crate) mod boringssl;
#[path = "providers/openssl.rs"]
pub(crate) mod openssl;
#[path = "providers/rustls.rs"]
pub(crate) mod rustls;
