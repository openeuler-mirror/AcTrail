//! TLS uprobe target tables.

pub(super) struct TlsUprobeTarget {
    pub(super) program: &'static str,
    pub(super) symbol: &'static str,
    pub(super) retprobe: bool,
}

pub(super) const OPENSSL_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
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

pub(super) const BORINGSSL_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
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
];

pub(super) const RUSTLS_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
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
