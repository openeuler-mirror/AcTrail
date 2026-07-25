//! TLS provider detection command.

use crate::ToolResult;
use crate::args::{DetectArgs, ProviderChoice, SourceChoice, require_arch};
use crate::binary::resolve_entry_elf;
use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TargetIdentity, TlsProvider};
use crate::probe_detector::contract::candidate::ProbeCandidate;
use crate::probe_detector::contract::detection::{
    DetectionEvidence, DetectionOutcome, DetectionRequest, ProbeConsumer, ProbeContext,
};
use crate::probe_detector::contract::detector::ProbeDetector;
use crate::probe_detector::detector::tls::{TlsProbeDetector, TlsProbeDetectorConfig};

use super::report::*;

pub(crate) fn run(args: DetectArgs) -> ToolResult<DetectReport> {
    let binary = resolve_entry_elf(&args.binary)?;
    let image = ElfImage::parse(&binary)?;
    require_arch(image.arch(), args.arch, image.path())?;
    let target = TargetIdentity {
        binary: image.path().to_path_buf(),
        architecture: image.arch().as_str().to_string(),
        identity: image.identity().clone(),
    };
    let request = DetectionRequest {
        requested_provider: provider(args.provider),
        requested_source: source(args.source),
        libraries: args.libraries,
        library_search_dirs: args.library_search_dirs,
        consumer: ProbeConsumer::PlanOnly,
    };
    let context = ProbeContext::executable(&target, &image, &request);
    let detector =
        TlsProbeDetector::try_new(TlsProbeDetectorConfig::for_diagnostics(args.match_limit))?;
    let outcome = detector
        .detect(&context)
        .map_err(|error| crate::ToolError::new(error.to_string()))?;
    let mut candidates = Vec::new();
    OutcomeProjector::new(&mut candidates).project(&outcome);
    Ok(DetectReport {
        target: TargetReport::from_image(&image),
        notices: Vec::new(),
        candidates,
    })
}

struct OutcomeProjector<'a> {
    reports: &'a mut Vec<CandidateReport>,
}

impl<'a> OutcomeProjector<'a> {
    fn new(reports: &'a mut Vec<CandidateReport>) -> Self {
        Self { reports }
    }

    fn project(&mut self, outcome: &DetectionOutcome) {
        match outcome {
            DetectionOutcome::Matched(candidate) => {
                if candidate.evidence.children.is_empty() {
                    self.reports.push(Self::matched(candidate));
                } else {
                    for child in &candidate.evidence.children {
                        self.project(child);
                    }
                }
            }
            DetectionOutcome::NoMatch(evidence) => self.project_no_match(evidence),
            DetectionOutcome::Inapplicable(_) => {}
            DetectionOutcome::Ambiguous(ambiguous) => {
                for candidate in &ambiguous.candidates {
                    let mut report = Self::matched(candidate);
                    report.status = CandidateStatus::Ambiguous;
                    self.reports.push(report);
                }
            }
            DetectionOutcome::Collected(evidence) => {
                for child in &evidence.children {
                    self.project(child);
                }
            }
        }
    }

    fn project_no_match(&mut self, evidence: &DetectionEvidence) {
        if !evidence.children.is_empty() {
            for child in &evidence.children {
                self.project(child);
            }
            return;
        }
        let provider = evidence.detector_path.display();
        self.reports.push(CandidateReport::failed(
            "auto",
            &provider,
            evidence
                .rejection
                .clone()
                .unwrap_or_else(|| "detector did not match".to_string()),
        ));
    }

    fn matched(candidate: &ProbeCandidate) -> CandidateReport {
        let mut report =
            CandidateReport::new(candidate.source.as_str(), candidate.provider.as_str());
        if candidate.source == ProbeSource::SharedLibrary {
            report.library = Some(LibraryReport {
                path: candidate.binary.path.display().to_string(),
                confidence: "detector-match".to_string(),
                note: None,
                architecture: Some(candidate.binary.architecture.clone()),
                identity: Some(candidate.binary.identity.clone()),
            });
        }
        report.endpoints = candidate
            .points
            .iter()
            .map(|point| EndpointReport {
                symbol: point.symbol.clone(),
                virtual_address: format!("0x{:x}", point.virtual_address),
                file_offset: format!("0x{:x}", point.file_offset),
            })
            .collect();
        if !candidate.evidence.patterns.is_empty() {
            report.pattern_matches = Some(PatternMatchesReport {
                arch: candidate.evidence.architecture.clone(),
                entries: candidate
                    .evidence
                    .patterns
                    .iter()
                    .map(|pattern| PatternMatchReport {
                        pattern_id: pattern.pattern_id.clone(),
                        symbol: pattern.symbol.clone(),
                        library: candidate.provider.as_str().to_string(),
                        resolver: candidate.resolver.clone(),
                        pattern_length: pattern.pattern_length.to_string(),
                        match_count: pattern.match_count,
                        matches: pattern
                            .shown_matches
                            .iter()
                            .map(|found| OffsetAddressReport {
                                file_offset: format!("0x{:x}", found.file_offset),
                                virtual_address: format!("0x{:x}", found.virtual_address),
                            })
                            .collect(),
                    })
                    .collect(),
            });
        }
        report.detected_offsets = candidate
            .points
            .iter()
            .map(|point| DetectedOffsetReport {
                symbol: point.symbol.clone(),
                file_offset: format!("0x{:x}", point.file_offset),
                virtual_address: format!("0x{:x}", point.virtual_address),
            })
            .collect();
        let consumer = &candidate.capability.consumer;
        report.runtime_config = vec![
            (
                "detector_path".to_string(),
                candidate.detector_path.display(),
            ),
            (
                "capability_architecture".to_string(),
                consumer.key.architecture.clone(),
            ),
            (
                "capability_provider".to_string(),
                consumer.key.provider.as_str().to_string(),
            ),
            (
                "capability_source".to_string(),
                consumer.key.source.as_str().to_string(),
            ),
            (
                "capability_resolver".to_string(),
                consumer.key.resolver.clone(),
            ),
            (
                "capability_consumer".to_string(),
                consumer.key.consumer.as_str().to_string(),
            ),
            (
                "capability_supported".to_string(),
                consumer.supported.to_string(),
            ),
            (
                "capability_validation".to_string(),
                consumer.validation_status.clone(),
            ),
        ];
        report.symbol_map = Some(SymbolMapReport {
            resolver: candidate.resolver.clone(),
            library: candidate.provider.as_str().to_string(),
            arch: candidate.binary.architecture.clone(),
            identity: candidate.binary.identity.clone(),
            symbols: candidate
                .points
                .iter()
                .map(|point| {
                    (
                        point.symbol.clone(),
                        format!("0x{:x}", point.virtual_address),
                    )
                })
                .collect(),
        });
        report.status = CandidateStatus::Matched;
        report
    }
}

fn provider(choice: ProviderChoice) -> Option<TlsProvider> {
    match choice {
        ProviderChoice::Auto => None,
        ProviderChoice::OpenSsl => Some(TlsProvider::OpenSsl),
        ProviderChoice::BoringSsl => Some(TlsProvider::BoringSsl),
        ProviderChoice::Rustls => Some(TlsProvider::Rustls),
        ProviderChoice::Go => Some(TlsProvider::Go),
        ProviderChoice::GnuTls => Some(TlsProvider::GnuTls),
        ProviderChoice::Nss => Some(TlsProvider::Nss),
    }
}

fn source(choice: SourceChoice) -> Option<ProbeSource> {
    match choice {
        SourceChoice::Auto => None,
        SourceChoice::Executable => Some(ProbeSource::Executable),
        SourceChoice::SharedLibrary => Some(ProbeSource::SharedLibrary),
    }
}
