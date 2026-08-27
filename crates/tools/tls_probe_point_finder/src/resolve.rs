//! Shared TLS probe-point resolution layer.
//!
//! CLI adapters (`detect`, `fast`) and the daemon plan adapter all drive the
//! same detector through this module.  CLI-facing code keeps its existing
//! report/plan projections, while the daemon consumes `ProbeResolution`
//! containing one or more complete plans.

use std::rc::Rc;

use crate::binary::resolve_entry_elf;
use crate::elf::{BinaryAnalysisCache, DEFAULT_LOW_MEMORY_CHUNK_BYTES, ElfImage, ScanMode};
use crate::fast::{FastProbeRequest, ProbeConsumer, require_arch};
use crate::plan::{ProbePointPlan, TargetIdentity};
use crate::probe_detector::contract::detection::{
    DetectionOutcome, DetectionRequest, ProbeContext,
};
use crate::probe_detector::contract::detector::ProbeDetector;
use crate::probe_detector::detector::tls::{TlsProbeDetector, TlsProbeDetectorConfig};
use crate::{ToolError, ToolResult};

/// How the shared detector result should be reduced for a consumer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveMode {
    /// Return the first complete plan (fast CLI and legacy daemon behavior).
    Single,
    /// Return every complete plan from the CollectAll detection pass.
    All,
}

/// Plans produced for a daemon/plan consumer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeResolution {
    pub plans: Vec<ProbePointPlan>,
}

/// Resolve all complete plans for a target.
pub fn resolve_plans(
    request: FastProbeRequest,
    consumer: ProbeConsumer,
    mode: ResolveMode,
) -> ToolResult<ProbeResolution> {
    resolve_plans_with_scan(request, consumer, ScanMode::Full, mode)
}

/// Resolve all complete plans with an explicit ELF scan mode.
pub fn resolve_plans_with_scan(
    request: FastProbeRequest,
    consumer: ProbeConsumer,
    scan: ScanMode,
    mode: ResolveMode,
) -> ToolResult<ProbeResolution> {
    resolve_plans_with_optional_cache(request, consumer, scan, mode, None)
}

pub fn resolve_plans_with_analysis_cache(
    request: FastProbeRequest,
    consumer: ProbeConsumer,
    mode: ResolveMode,
    cache: Rc<BinaryAnalysisCache>,
) -> ToolResult<ProbeResolution> {
    resolve_plans_with_optional_cache(request, consumer, ScanMode::Full, mode, Some(cache))
}

fn resolve_plans_with_optional_cache(
    request: FastProbeRequest,
    consumer: ProbeConsumer,
    scan: ScanMode,
    mode: ResolveMode,
    cache: Option<Rc<BinaryAnalysisCache>>,
) -> ToolResult<ProbeResolution> {
    let outcome =
        detect_outcome_with_cache(&request, consumer, scan, mode == ResolveMode::All, cache)?;
    let plans = match outcome {
        DetectionOutcome::Collected(evidence) => {
            let mut plans = Vec::new();
            for child in &evidence.children {
                collect_plans(child, &mut plans);
            }
            plans
        }
        DetectionOutcome::Matched(candidate) => vec![candidate.into_plan()],
        DetectionOutcome::Ambiguous(ambiguous) => {
            return Err(ToolError::new(format!(
                "ambiguous TLS probe detection: {}",
                ambiguous
                    .candidates
                    .iter()
                    .map(|candidate| candidate.detector_path.display())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        DetectionOutcome::Inapplicable(_) | DetectionOutcome::NoMatch(_) => {
            return Err(ToolError::new(
                "no supported TLS payload probe points found",
            ));
        }
    };
    Ok(ProbeResolution { plans })
}

/// Run the shared detector core.
///
/// `collect_all` selects the diagnostics configuration (CollectAll); otherwise
/// the legacy FirstComplete configuration is used.  Both CLI entry points and
/// the daemon plan adapter funnel through this function.
pub(crate) fn detect_outcome(
    request: &FastProbeRequest,
    consumer: ProbeConsumer,
    scan: ScanMode,
    collect_all: bool,
) -> ToolResult<DetectionOutcome> {
    detect_outcome_with_cache(request, consumer, scan, collect_all, None)
}

fn detect_outcome_with_cache(
    request: &FastProbeRequest,
    consumer: ProbeConsumer,
    scan: ScanMode,
    collect_all: bool,
    cache: Option<Rc<BinaryAnalysisCache>>,
) -> ToolResult<DetectionOutcome> {
    let binary = resolve_entry_elf(&request.binary)?;
    let image = match cache {
        Some(cache) => ElfImage::parse_with_analysis_cache(
            &binary,
            scan,
            DEFAULT_LOW_MEMORY_CHUNK_BYTES,
            cache,
        )?,
        None => ElfImage::parse_with_mode(&binary, scan, DEFAULT_LOW_MEMORY_CHUNK_BYTES)?,
    };
    require_arch(image.arch(), request.arch, image.path())?;
    let target = TargetIdentity {
        binary: image.path().to_path_buf(),
        architecture: image.arch().as_str().to_string(),
        identity: image.identity().clone(),
    };
    let detection_request = DetectionRequest {
        requested_provider: request.provider.requested_provider(),
        requested_source: request.source.requested_source(),
        libraries: request.libraries.clone(),
        library_search_dirs: request.library_search_dirs.clone(),
        consumer,
    };
    let context = ProbeContext::executable(&target, &image, &detection_request);
    let detector = if collect_all {
        TlsProbeDetector::try_new(TlsProbeDetectorConfig::for_diagnostics(request.match_limit))?
    } else {
        TlsProbeDetector::try_new(TlsProbeDetectorConfig::with_match_limit(
            request.match_limit,
        ))?
    };
    detector
        .detect(&context)
        .map_err(|error| ToolError::new(error.to_string()))
}

fn collect_plans(outcome: &DetectionOutcome, plans: &mut Vec<ProbePointPlan>) {
    match outcome {
        DetectionOutcome::Matched(candidate) => {
            plans.push(candidate.clone().into_plan());
        }
        DetectionOutcome::Collected(evidence) => {
            for child in &evidence.children {
                collect_plans(child, plans);
            }
        }
        DetectionOutcome::Ambiguous(ambiguous) => {
            for candidate in &ambiguous.candidates {
                plans.push(candidate.clone().into_plan());
            }
        }
        DetectionOutcome::Inapplicable(_) | DetectionOutcome::NoMatch(_) => {}
    }
}
