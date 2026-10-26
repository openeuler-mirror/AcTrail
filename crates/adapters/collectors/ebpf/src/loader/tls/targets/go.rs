//! Go TLS entries and plaintext read copy point.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const GO_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
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
