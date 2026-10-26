//! BoringSSL provider entries and shared SSL completion programs.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const BORINGSSL_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_boringssl_write_enter",
        symbol: "SSL_write",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_write_exit",
        symbol: "SSL_write",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_boringssl_read_enter",
        symbol: "SSL_read",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_read_exit",
        symbol: "SSL_read",
        retprobe: true,
    },
];
