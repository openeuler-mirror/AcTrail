//! TLS payload diagnostic event logging.

use config_core::daemon::DiagnosticLogLevel;
use ebpf_collector::TlsDiagnosticEvent;

use crate::services::attach::SqliteAttachService;

const TLS_DIAG_REASON_TRACE_LOOKUP_MISS: u32 = 1;
const TLS_DIAG_REASON_TRACE_LOOKUP_HOST_FALLBACK: u32 = 2;
const TLS_DIAG_REASON_EMPTY_BUFFER: u32 = 3;
const TLS_DIAG_REASON_PENDING_UPDATE_FAIL: u32 = 4;
const TLS_DIAG_REASON_PENDING_NAMESPACE_UPDATE_FAIL: u32 = 5;
const TLS_DIAG_REASON_COMPLETION_MISSING_PENDING: u32 = 6;

const TLS_DIRECTION_OUTBOUND: u32 = 1;
const TLS_DIRECTION_INBOUND: u32 = 2;

const TLS_SYMBOL_SSL_WRITE: u32 = 1;
const TLS_SYMBOL_SSL_READ: u32 = 2;
const TLS_SYMBOL_SSL_WRITE_EX: u32 = 3;
const TLS_SYMBOL_SSL_READ_EX: u32 = 4;
const TLS_SYMBOL_RUSTLS_WRITE: u32 = 5;
const TLS_SYMBOL_RUSTLS_WRITE_VECTORED: u32 = 6;

const TLS_LIBRARY_OPENSSL: u32 = 1;
const TLS_LIBRARY_BORINGSSL: u32 = 2;
const TLS_LIBRARY_RUSTLS: u32 = 3;

impl SqliteAttachService {
    pub(super) fn log_tls_diagnostic_events_impl(&mut self) {
        let events = self.collector.take_tls_diagnostic_events();
        if events.is_empty() || !self.diagnostic_log_enabled(DiagnosticLogLevel::Debug) {
            return;
        }
        for event in events {
            self.log_tls_diagnostic_event(&event);
        }
    }

    fn log_tls_diagnostic_event(&self, event: &TlsDiagnosticEvent) {
        self.log_diagnostic(
            DiagnosticLogLevel::Debug,
            format_args!(
                "tls_payload_event reason={} host_pid={} host_tid={} namespace_pid={} namespace_tid={} comm={} direction={} library={} symbol={} requested_size={} buffer_ptr=0x{:x} lookup_flags=0x{:x}",
                tls_diag_reason(event.reason),
                event.host_tgid,
                event.host_tid,
                event.namespace_tgid,
                event.namespace_tid,
                event.comm,
                tls_direction(event.direction),
                tls_library(event.library),
                tls_symbol(event.symbol),
                event.requested_size,
                event.buffer_ptr,
                event.lookup_flags,
            ),
        );
    }
}

fn tls_diag_reason(raw: u32) -> &'static str {
    match raw {
        TLS_DIAG_REASON_TRACE_LOOKUP_MISS => "trace_lookup_miss",
        TLS_DIAG_REASON_TRACE_LOOKUP_HOST_FALLBACK => "trace_lookup_host_fallback",
        TLS_DIAG_REASON_EMPTY_BUFFER => "empty_buffer",
        TLS_DIAG_REASON_PENDING_UPDATE_FAIL => "pending_update_fail",
        TLS_DIAG_REASON_PENDING_NAMESPACE_UPDATE_FAIL => "pending_namespace_update_fail",
        TLS_DIAG_REASON_COMPLETION_MISSING_PENDING => "completion_missing_pending",
        _ => "unknown",
    }
}

fn tls_direction(raw: u32) -> &'static str {
    match raw {
        TLS_DIRECTION_OUTBOUND => "outbound",
        TLS_DIRECTION_INBOUND => "inbound",
        _ => "unknown",
    }
}

fn tls_symbol(raw: u32) -> &'static str {
    match raw {
        TLS_SYMBOL_SSL_WRITE => "SSL_write",
        TLS_SYMBOL_SSL_READ => "SSL_read",
        TLS_SYMBOL_SSL_WRITE_EX => "SSL_write_ex",
        TLS_SYMBOL_SSL_READ_EX => "SSL_read_ex",
        TLS_SYMBOL_RUSTLS_WRITE => "rustls_write",
        TLS_SYMBOL_RUSTLS_WRITE_VECTORED => "rustls_write_vectored",
        _ => "unknown",
    }
}

fn tls_library(raw: u32) -> &'static str {
    match raw {
        TLS_LIBRARY_OPENSSL => "openssl",
        TLS_LIBRARY_BORINGSSL => "boringssl",
        TLS_LIBRARY_RUSTLS => "rustls",
        _ => "unknown",
    }
}
