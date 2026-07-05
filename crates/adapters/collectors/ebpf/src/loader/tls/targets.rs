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

pub(super) const GO_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_go_tls_write_enter",
        symbol: tls_probe_point_finder::GO_TLS_WRITE_SYMBOL,
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_go_tls_conn_read_enter",
        symbol: tls_probe_point_finder::GO_TLS_READ_SYMBOL,
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_go_tls_memmove_enter",
        symbol: tls_probe_point_finder::GO_RUNTIME_MEMMOVE_SYMBOL,
        retprobe: false,
    },
];

pub(super) const GNUTLS_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_gnutls_record_send_enter",
        symbol: "gnutls_record_send",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_gnutls_record_send_exit",
        symbol: "gnutls_record_send",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_gnutls_record_recv_enter",
        symbol: "gnutls_record_recv",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_gnutls_record_recv_exit",
        symbol: "gnutls_record_recv",
        retprobe: true,
    },
];

pub(super) const NSS_NSPR_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_nspr_pr_write_enter",
        symbol: "PR_Write",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_write_exit",
        symbol: "PR_Write",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_send_enter",
        symbol: "PR_Send",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_send_exit",
        symbol: "PR_Send",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_read_enter",
        symbol: "PR_Read",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_read_exit",
        symbol: "PR_Read",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_recv_enter",
        symbol: "PR_Recv",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_recv_exit",
        symbol: "PR_Recv",
        retprobe: true,
    },
];
