//! OpenSSL plaintext entry and completion programs.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const OPENSSL_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_ssl_write_enter",
        symbol: "SSL_write",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_write_exit",
        symbol: "SSL_write",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_ssl_read_enter",
        symbol: "SSL_read",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_read_exit",
        symbol: "SSL_read",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_ssl_write_ex_enter",
        symbol: "SSL_write_ex",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_write_ex_exit",
        symbol: "SSL_write_ex",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_ssl_read_ex_enter",
        symbol: "SSL_read_ex",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_ssl_read_ex_exit",
        symbol: "SSL_read_ex",
        retprobe: true,
    },
];
