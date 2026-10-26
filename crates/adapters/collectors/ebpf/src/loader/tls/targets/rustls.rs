//! Rustls public plaintext write programs.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const RUSTLS_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_rustls_write_enter",
        symbol: "rustls_plaintext_write",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_rustls_write_exit",
        symbol: "rustls_plaintext_write",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_rustls_write_vectored_enter",
        symbol: "rustls_plaintext_write_vectored",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_rustls_write_vectored_exit",
        symbol: "rustls_plaintext_write_vectored",
        retprobe: true,
    },
];
