use std::collections::BTreeMap;

use crate::ToolResult;
use crate::plan::{
    AttachPoint, CaptureStrategy, PayloadDirection, ProbeBinary, ProbePoint, TlsProvider,
};
use crate::probe_detector::contract::candidate::ProbeCandidate;
use crate::probe_detector::contract::candidate::closure::ProbeClosure;
use crate::probe_detector::contract::capability::{
    CapabilityKey, ConsumerCapability, DetectorCapability,
};
use crate::probe_detector::contract::detection::{DetectionEvidence, EvidenceFact, ProbeContext};
use crate::probe_detector::contract::identity::DetectorPath;

use super::{gnutls, go_tls, nss, openssl, rustls};

pub(super) struct TlsProbeCandidateFactory<'a, 'context> {
    context: &'a ProbeContext<'context>,
    detector_path: DetectorPath,
    provider: TlsProvider,
    resolver: &'static str,
}

impl<'a, 'context> TlsProbeCandidateFactory<'a, 'context> {
    pub(super) fn new(
        context: &'a ProbeContext<'context>,
        detector_path: DetectorPath,
        provider: TlsProvider,
        resolver: &'static str,
    ) -> Self {
        Self {
            context,
            detector_path,
            provider,
            resolver,
        }
    }

    pub(super) fn from_symbols(
        self,
        symbols: &BTreeMap<String, u64>,
        evidence: DetectionEvidence,
    ) -> ToolResult<ProbeCandidate> {
        let points = symbols
            .iter()
            .map(|(symbol, virtual_address)| {
                Ok(ProbePoint {
                    symbol: symbol.clone(),
                    direction: Self::direction(symbol),
                    attach: Self::attach(symbol),
                    capture: Self::capture(symbol),
                    virtual_address: *virtual_address,
                    file_offset: self
                        .context
                        .probe
                        .image
                        .file_offset_for_virtual_address(*virtual_address)?,
                })
            })
            .collect::<ToolResult<Vec<_>>>()?;
        Ok(self.from_points(points, evidence))
    }

    pub(super) fn from_offsets(
        self,
        offsets: impl IntoIterator<Item = (String, u64, u64)>,
        evidence: DetectionEvidence,
    ) -> ProbeCandidate {
        let points = offsets
            .into_iter()
            .map(|(symbol, virtual_address, file_offset)| ProbePoint {
                direction: Self::direction(&symbol),
                attach: Self::attach(&symbol),
                capture: Self::capture(&symbol),
                symbol,
                virtual_address,
                file_offset,
            })
            .collect();
        self.from_points(points, evidence)
    }

    fn from_points(
        self,
        points: Vec<ProbePoint>,
        mut evidence: DetectionEvidence,
    ) -> ProbeCandidate {
        if let Some(library) = self.context.probe.library {
            evidence.facts.push(EvidenceFact {
                key: "library_path".to_string(),
                value: library.path.display().to_string(),
            });
            if let Some(note) = &library.note {
                evidence.facts.push(EvidenceFact {
                    key: "library_note".to_string(),
                    value: note.clone(),
                });
            }
        }
        let complete_plaintext_closure = ProbeClosure::from_points(&points).is_some();
        let image = self.context.probe.image;
        let capability_key = CapabilityKey {
            architecture: image.arch().as_str().to_string(),
            provider: self.provider,
            source: self.context.probe.source,
            resolver: self.resolver.to_string(),
            consumer: self.context.request.consumer,
        };
        let consumer = ConsumerCapability::evaluate(
            capability_key,
            &points,
            complete_plaintext_closure,
            self.context.target_image.has_interpreter(),
        );
        ProbeCandidate {
            detector_path: self.detector_path,
            target: self.context.target.clone(),
            provider: self.provider,
            source: self.context.probe.source,
            binary: ProbeBinary {
                path: image.path().to_path_buf(),
                architecture: image.arch().as_str().to_string(),
                identity: image.identity().clone(),
            },
            resolver: self.resolver.to_string(),
            points,
            evidence,
            capability: DetectorCapability {
                complete_plaintext_closure,
                consumer,
            },
        }
    }

    fn direction(symbol: &str) -> PayloadDirection {
        match symbol {
            rustls::RUNTIME_BUFFER_PLAINTEXT_SYMBOL
            | openssl::SSL_WRITE
            | openssl::SSL_WRITE_EX
            | openssl::SSL_WRITE_EX2
            | gnutls::RECORD_SEND
            | nss::NSPR_PR_WRITE
            | nss::NSPR_PR_SEND
            | go_tls::WRITE_SYMBOL => PayloadDirection::Outbound,
            rustls::RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL
            | openssl::SSL_READ
            | openssl::SSL_READ_EX
            | "SSL_read_internal"
            | gnutls::RECORD_RECV
            | nss::NSPR_PR_READ
            | nss::NSPR_PR_RECV
            | go_tls::RUNTIME_MEMMOVE_SYMBOL => PayloadDirection::Inbound,
            _ => PayloadDirection::Control,
        }
    }

    fn attach(symbol: &str) -> AttachPoint {
        if matches!(
            symbol,
            gnutls::RECORD_SEND
                | gnutls::RECORD_RECV
                | nss::NSPR_PR_WRITE
                | nss::NSPR_PR_SEND
                | nss::NSPR_PR_READ
                | nss::NSPR_PR_RECV
        ) {
            return AttachPoint::Return;
        }
        if Self::direction(symbol) == PayloadDirection::Inbound
            && matches!(
                symbol,
                openssl::SSL_READ
                    | openssl::SSL_READ_EX
                    | "SSL_read_internal"
                    | go_tls::READ_SYMBOL
            )
        {
            AttachPoint::Return
        } else {
            AttachPoint::Entry
        }
    }

    fn capture(symbol: &str) -> CaptureStrategy {
        match Self::attach(symbol) {
            AttachPoint::Entry => CaptureStrategy::EntryBuffer,
            AttachPoint::Return => CaptureStrategy::ReturnBufferFromEntryState,
        }
    }
}
