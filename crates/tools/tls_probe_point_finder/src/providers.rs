//! TLS provider detector modules.

#[path = "providers/boringssl.rs"]
pub(crate) mod boringssl;
#[path = "providers/go_tls.rs"]
pub(crate) mod go_tls;
#[path = "providers/legacy_tls.rs"]
pub(crate) mod legacy_tls;
#[path = "providers/openssl.rs"]
pub(crate) mod openssl;
#[path = "providers/rustls.rs"]
pub(crate) mod rustls;
