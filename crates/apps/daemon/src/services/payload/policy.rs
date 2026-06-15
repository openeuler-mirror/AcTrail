use config_core::daemon::{PayloadRedactionPolicy, PayloadStdioStorageMode};
use control_contract::reply::ControlError;
use model_core::payload::PayloadSourceBoundary;
use payload_event::RawPayloadSegment;

#[derive(Clone, Copy)]
pub(in crate::services::payload) struct PayloadPolicyConfig {
    pub(in crate::services::payload) tls_enabled: bool,
    pub(in crate::services::payload) tls_redaction_policy: PayloadRedactionPolicy,
    pub(in crate::services::payload) tls_retention_max_bytes_per_trace: u64,
    pub(in crate::services::payload) stdio_enabled: bool,
    pub(in crate::services::payload) stdio_redaction_policy: PayloadRedactionPolicy,
    pub(in crate::services::payload) stdio_retention_max_bytes_per_trace: u64,
    pub(in crate::services::payload) stdio_stdin_storage_mode: PayloadStdioStorageMode,
    pub(in crate::services::payload) stdio_stdout_storage_mode: PayloadStdioStorageMode,
    pub(in crate::services::payload) stdio_stderr_storage_mode: PayloadStdioStorageMode,
    pub(in crate::services::payload) socket_enabled: bool,
    pub(in crate::services::payload) socket_redaction_policy: PayloadRedactionPolicy,
    pub(in crate::services::payload) socket_retention_max_bytes_per_trace: u64,
}

impl PayloadPolicyConfig {
    pub(in crate::services::payload) fn for_segment(
        &self,
        raw: &RawPayloadSegment,
    ) -> Result<PayloadProcessingPolicy, ControlError> {
        match raw.source_boundary {
            PayloadSourceBoundary::TlsUserSpace => {
                if !self.tls_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "TLS payload segment received while payload_tls_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.tls_redaction_policy,
                    retention_max_bytes_per_trace: self.tls_retention_max_bytes_per_trace,
                    stdio_storage_mode: PayloadStdioStorageMode::Full,
                })
            }
            PayloadSourceBoundary::Stdio => {
                if !self.stdio_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "stdio payload segment received while payload_stdio_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.stdio_redaction_policy,
                    retention_max_bytes_per_trace: self.stdio_retention_max_bytes_per_trace,
                    stdio_storage_mode: self.stdio_storage_mode(raw)?,
                })
            }
            PayloadSourceBoundary::Syscall => {
                if !self.socket_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "socket payload segment received while payload_socket_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.socket_redaction_policy,
                    retention_max_bytes_per_trace: self.socket_retention_max_bytes_per_trace,
                    stdio_storage_mode: PayloadStdioStorageMode::Full,
                })
            }
        }
    }

    fn stdio_storage_mode(
        &self,
        raw: &RawPayloadSegment,
    ) -> Result<PayloadStdioStorageMode, ControlError> {
        match raw.protocol_hint.as_deref() {
            Some("stdin") => Ok(self.stdio_stdin_storage_mode),
            Some("stdout") => Ok(self.stdio_stdout_storage_mode),
            Some("stderr") => Ok(self.stdio_stderr_storage_mode),
            Some(stream) => Err(ControlError::new(
                "payload_policy",
                format!("unsupported stdio payload stream {stream}"),
            )),
            None => Err(ControlError::new(
                "payload_policy",
                "stdio payload segment missing protocol_hint",
            )),
        }
    }
}

pub(in crate::services::payload) struct PayloadProcessingPolicy {
    pub(in crate::services::payload) redaction: PayloadRedactionPolicy,
    pub(in crate::services::payload) retention_max_bytes_per_trace: u64,
    pub(in crate::services::payload) stdio_storage_mode: PayloadStdioStorageMode,
}
