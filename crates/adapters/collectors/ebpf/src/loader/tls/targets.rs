//! Shared TLS uprobe target representation and provider tables.

#[path = "targets/boringssl.rs"]
mod boringssl;
#[path = "targets/gnutls.rs"]
mod gnutls;
#[path = "targets/go.rs"]
mod go;
#[path = "targets/nss.rs"]
mod nss;
#[path = "targets/openssl.rs"]
mod openssl;
#[path = "targets/rustls.rs"]
mod rustls;

pub(super) use boringssl::BORINGSSL_UPROBE_TARGETS;
pub(super) use gnutls::GNUTLS_UPROBE_TARGETS;
pub(super) use go::GO_UPROBE_TARGETS;
pub(super) use nss::NSS_NSPR_UPROBE_TARGETS;
pub(super) use openssl::OPENSSL_UPROBE_TARGETS;
pub(super) use rustls::RUSTLS_UPROBE_TARGETS;

pub(super) struct TlsUprobeTarget {
    pub(super) program: &'static str,
    pub(super) symbol: &'static str,
    pub(super) retprobe: bool,
}
