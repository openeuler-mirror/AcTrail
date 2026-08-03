use crate::plan::{ProbePoint, TlsProvider};
use crate::probe_detector::contract::capability::CapabilityKey;
use crate::probe_detector::contract::detection::ProbeConsumer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConsumerCapability {
    pub(crate) key: CapabilityKey,
    pub(crate) supported: bool,
    pub(crate) validation_status: String,
}

impl ConsumerCapability {
    pub(crate) fn evaluate(
        key: CapabilityKey,
        points: &[ProbePoint],
        complete_plaintext_closure: bool,
        target_has_interpreter: bool,
    ) -> Self {
        let rejection = if !complete_plaintext_closure {
            Some("incomplete-plaintext-closure".to_string())
        } else if key.consumer != ProbeConsumer::PlanOnly
            && key.architecture != std::env::consts::ARCH
        {
            Some(format!(
                "consumer-architecture-mismatch:{}",
                key.architecture
            ))
        } else {
            Self::consumer_rejection(&key, points, target_has_interpreter)
        };
        Self {
            supported: rejection.is_none(),
            validation_status: rejection
                .unwrap_or_else(|| "consumer-structure-validated".to_string()),
            key,
        }
    }

    fn consumer_rejection(
        key: &CapabilityKey,
        points: &[ProbePoint],
        target_has_interpreter: bool,
    ) -> Option<String> {
        match key.consumer {
            ProbeConsumer::PlanOnly => None,
            ProbeConsumer::Standalone => {
                if key.provider == TlsProvider::Go {
                    return Some("standalone-does-not-support-go".to_string());
                }
                Self::first_unsupported_symbol(points, STANDALONE_SYMBOLS, "standalone")
            }
            ProbeConsumer::Sync => Self::sync_rejection(key.provider, points),
            ProbeConsumer::Daemon if target_has_interpreter => {
                Self::sync_rejection(key.provider, points)
            }
            ProbeConsumer::Daemon => {
                if !matches!(key.provider, TlsProvider::OpenSsl | TlsProvider::Rustls) {
                    return Some(format!(
                        "daemon-direct-does-not-support-provider:{}",
                        key.provider.as_str()
                    ));
                }
                Self::first_unsupported_symbol(points, DAEMON_DIRECT_SYMBOLS, "daemon-direct")
            }
        }
    }

    fn sync_rejection(provider: TlsProvider, points: &[ProbePoint]) -> Option<String> {
        if !matches!(
            provider,
            TlsProvider::OpenSsl | TlsProvider::BoringSsl | TlsProvider::Rustls
        ) {
            return Some(format!(
                "sync-does-not-support-provider:{}",
                provider.as_str()
            ));
        }
        Self::first_unsupported_symbol(points, SYNC_SYMBOLS, "sync")
    }

    fn first_unsupported_symbol(
        points: &[ProbePoint],
        supported: &[&str],
        consumer: &str,
    ) -> Option<String> {
        points
            .iter()
            .find(|point| !supported.contains(&point.symbol.as_str()))
            .map(|point| format!("{consumer}-unsupported-symbol:{}", point.symbol))
    }
}

const STANDALONE_SYMBOLS: &[&str] = &[
    "SSL_write",
    "SSL_read",
    "SSL_write_ex",
    "SSL_write_ex2",
    "SSL_read_ex",
    "SSL_read_internal",
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
    "gnutls_record_send",
    "gnutls_record_recv",
    "PR_Write",
    "PR_Send",
    "PR_Read",
    "PR_Recv",
];

const SYNC_SYMBOLS: &[&str] = &[
    "SSL_write",
    "SSL_write_ex",
    "SSL_write_ex2",
    "SSL_read",
    "SSL_read_ex",
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
];

const DAEMON_DIRECT_SYMBOLS: &[&str] = &[
    "SSL_write",
    "SSL_write_ex",
    "SSL_read",
    "SSL_read_ex",
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
];
