//! GnuTLS record entry and completion programs.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const GNUTLS_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
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
