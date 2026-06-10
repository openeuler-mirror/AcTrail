//! Fast probe-point resolution for payload capture startup.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::binary::resolve_entry_elf;
use crate::elf::{Arch, ElfImage};
use crate::plan::{
    AttachPoint, CaptureStrategy, PayloadDirection, ProbeBinary, ProbePoint, ProbePointPlan,
    ProbeSource, TargetIdentity, TlsProvider,
};
use crate::providers::{boringssl, go_tls, openssl, rustls};
use crate::{ToolError, ToolResult};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FastProbeRequest {
    pub binary: PathBuf,
    pub arch: ArchFilter,
    pub provider: ProviderFilter,
    pub source: SourceFilter,
    pub match_limit: usize,
    pub libraries: Vec<PathBuf>,
    pub library_search_dirs: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchFilter {
    Auto,
    Aarch64,
    X86_64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFilter {
    Auto,
    OpenSsl,
    BoringSsl,
    Rustls,
    Go,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFilter {
    Auto,
    Executable,
    SharedLibrary,
}

pub fn resolve(request: FastProbeRequest) -> ToolResult<ProbePointPlan> {
    let binary = resolve_entry_elf(&request.binary)?;
    let image = ElfImage::parse(&binary)?;
    require_arch(image.arch(), request.arch, image.path())?;

    if let Some(plan) = resolve_executable_symbols(&image, &request)? {
        return Ok(plan);
    }
    if let Some(plan) = resolve_direct_shared_library(&image, &request)? {
        return Ok(plan);
    }
    if let Some(plan) = resolve_recursive_shared_library(&image, &request)? {
        return Ok(plan);
    }
    if let Some(plan) = resolve_executable_go(&image, &request)? {
        return Ok(plan);
    }
    if let Some(plan) = resolve_static_patterns(&image, &request)? {
        return Ok(plan);
    }
    Err(ToolError::new(
        "no supported TLS payload probe points found",
    ))
}

fn resolve_executable_symbols(
    image: &ElfImage,
    request: &FastProbeRequest,
) -> ToolResult<Option<ProbePointPlan>> {
    if !request.source.allows_executable() {
        return Ok(None);
    }
    if request.provider.allows(TlsProvider::Rustls) {
        if let Some(symbols) = rustls::resolve_demangled_plaintext_symbols(image)? {
            let plan = plan_from_symbol_map(
                image,
                TlsProvider::Rustls,
                ProbeSource::Executable,
                rustls::RESOLVER,
                &symbols.runtime_symbols,
            )?;
            if plan.has_payload_closure() {
                return Ok(Some(plan));
            }
        }
    }
    if request.provider.allows(TlsProvider::OpenSsl) {
        let symbols = image.unique_defined_symbol_values(openssl::SYMBOLS)?;
        if has_all(&symbols, openssl::SYMBOLS) {
            let plan = plan_from_symbol_map(
                image,
                TlsProvider::OpenSsl,
                ProbeSource::Executable,
                openssl::RESOLVER,
                &symbols,
            )?;
            if plan.has_payload_closure() {
                return Ok(Some(plan));
            }
        }
    }
    if request.provider == ProviderFilter::BoringSsl {
        let symbols = image.unique_defined_symbol_values(boringssl::map_symbols(image.arch()))?;
        if has_all(&symbols, boringssl::map_symbols(image.arch())) {
            let probe_symbols = boringssl_probe_symbols(&symbols);
            let plan = plan_from_symbol_map(
                image,
                TlsProvider::BoringSsl,
                ProbeSource::Executable,
                boringssl::SYMBOL_MAP_RESOLVER,
                &probe_symbols,
            )?;
            if plan.has_payload_closure() {
                return Ok(Some(plan));
            }
        }
    }
    Ok(None)
}

fn resolve_executable_go(
    image: &ElfImage,
    request: &FastProbeRequest,
) -> ToolResult<Option<ProbePointPlan>> {
    if !request.source.allows_executable() || !request.provider.allows(TlsProvider::Go) {
        return Ok(None);
    }
    let Some(symbols) = go_tls::resolve_pclntab_symbols(image, go_tls::SYMBOLS)? else {
        return Ok(None);
    };
    let plan = plan_from_symbol_map(
        image,
        TlsProvider::Go,
        ProbeSource::Executable,
        go_tls::RESOLVER,
        &symbols,
    )?;
    Ok(plan.has_payload_closure().then_some(plan))
}

fn resolve_direct_shared_library(
    image: &ElfImage,
    request: &FastProbeRequest,
) -> ToolResult<Option<ProbePointPlan>> {
    if !request.source.allows_shared_library() || !request.provider.allows(TlsProvider::OpenSsl) {
        return Ok(None);
    }
    let search = openssl::direct_library_candidates(
        image,
        &request.libraries,
        &request.library_search_dirs,
    )?;
    resolve_first_openssl_library(image, search.candidates)
}

fn resolve_static_patterns(
    image: &ElfImage,
    request: &FastProbeRequest,
) -> ToolResult<Option<ProbePointPlan>> {
    if !request.source.allows_executable() {
        return Ok(None);
    }
    if request.provider.allows(TlsProvider::Rustls) {
        match rustls::detect_static_patterns(image, request.match_limit) {
            Ok(detection) => {
                let plan = plan_from_detected_offsets(
                    image,
                    TlsProvider::Rustls,
                    ProbeSource::Executable,
                    rustls::RESOLVER,
                    detection
                        .offsets
                        .iter()
                        .map(|offset| {
                            (
                                offset.symbol.to_string(),
                                offset.virtual_address,
                                offset.file_offset as u64,
                            )
                        })
                        .collect(),
                )?;
                if plan.has_payload_closure() {
                    return Ok(Some(plan));
                }
            }
            Err(error) if request.provider == ProviderFilter::Rustls => return Err(error),
            Err(_) => {}
        }
    }
    if request.provider.allows(TlsProvider::BoringSsl) {
        match boringssl::detect_static_patterns(image, request.match_limit) {
            Ok(detection) => {
                let plan = plan_from_detected_offsets(
                    image,
                    TlsProvider::BoringSsl,
                    ProbeSource::Executable,
                    boringssl::STATIC_RESOLVER,
                    boringssl_static_probe_offsets(&detection),
                )?;
                if plan.has_payload_closure() {
                    return Ok(Some(plan));
                }
            }
            Err(error) if request.provider == ProviderFilter::BoringSsl => return Err(error),
            Err(_) => {}
        }
    }
    Ok(None)
}

fn boringssl_probe_symbols(symbols: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    symbols
        .iter()
        .filter(|(symbol, _)| is_boringssl_payload_symbol(symbol))
        .map(|(symbol, address)| (symbol.clone(), *address))
        .collect()
}

fn boringssl_static_probe_offsets(
    detection: &boringssl::StaticPatternDetection,
) -> Vec<(String, u64, u64)> {
    detection
        .offsets
        .iter()
        .filter(|offset| is_boringssl_payload_symbol(offset.symbol))
        .map(|offset| {
            (
                offset.symbol.to_string(),
                offset.virtual_address,
                offset.file_offset as u64,
            )
        })
        .collect()
}

fn is_boringssl_payload_symbol(symbol: &str) -> bool {
    matches!(symbol, "SSL_read" | "SSL_write")
}

fn resolve_recursive_shared_library(
    image: &ElfImage,
    request: &FastProbeRequest,
) -> ToolResult<Option<ProbePointPlan>> {
    if !request.source.allows_shared_library() || !request.provider.allows(TlsProvider::OpenSsl) {
        return Ok(None);
    }
    let search =
        openssl::library_candidates(image, &request.libraries, &request.library_search_dirs)?;
    resolve_first_openssl_library(image, search.candidates)
}

fn resolve_first_openssl_library(
    target: &ElfImage,
    candidates: Vec<openssl::LibraryCandidate>,
) -> ToolResult<Option<ProbePointPlan>> {
    for candidate in candidates {
        let library = ElfImage::parse(&candidate.path)?;
        if library.arch() != target.arch() {
            continue;
        }
        let symbols = library.unique_defined_symbol_values(openssl::SYMBOLS)?;
        if !has_all(&symbols, openssl::SYMBOLS) {
            continue;
        }
        let plan = plan_from_symbol_map(
            &library,
            TlsProvider::OpenSsl,
            ProbeSource::SharedLibrary,
            openssl::RESOLVER,
            &symbols,
        )?
        .with_target(target);
        if plan.has_payload_closure() {
            return Ok(Some(plan));
        }
    }
    Ok(None)
}

fn plan_from_symbol_map(
    image: &ElfImage,
    provider: TlsProvider,
    source: ProbeSource,
    resolver: &str,
    symbols: &BTreeMap<String, u64>,
) -> ToolResult<ProbePointPlan> {
    let mut points = Vec::new();
    for (symbol, virtual_address) in symbols {
        points.push(ProbePoint {
            symbol: symbol.clone(),
            direction: direction_for_symbol(symbol),
            attach: attach_for_symbol(symbol),
            capture: capture_for_symbol(symbol),
            virtual_address: *virtual_address,
            file_offset: image.file_offset_for_virtual_address(*virtual_address)?,
        });
    }
    Ok(ProbePointPlan {
        target: target_identity(image),
        provider,
        source,
        resolver: resolver.to_string(),
        binary: probe_binary(image),
        points,
    })
}

fn plan_from_detected_offsets(
    image: &ElfImage,
    provider: TlsProvider,
    source: ProbeSource,
    resolver: &str,
    offsets: Vec<(String, u64, u64)>,
) -> ToolResult<ProbePointPlan> {
    let points = offsets
        .into_iter()
        .map(|(symbol, virtual_address, file_offset)| ProbePoint {
            direction: direction_for_symbol(&symbol),
            attach: attach_for_symbol(&symbol),
            capture: capture_for_symbol(&symbol),
            symbol,
            virtual_address,
            file_offset,
        })
        .collect();
    Ok(ProbePointPlan {
        target: target_identity(image),
        provider,
        source,
        resolver: resolver.to_string(),
        binary: probe_binary(image),
        points,
    })
}

fn has_all(symbols: &BTreeMap<String, u64>, required: &[&str]) -> bool {
    required.iter().all(|symbol| symbols.contains_key(*symbol))
}

fn direction_for_symbol(symbol: &str) -> PayloadDirection {
    match symbol {
        rustls::RUNTIME_BUFFER_PLAINTEXT_SYMBOL | "SSL_write" | "SSL_write_ex" => {
            PayloadDirection::Outbound
        }
        go_tls::WRITE_SYMBOL => PayloadDirection::Outbound,
        go_tls::RUNTIME_MEMMOVE_SYMBOL => PayloadDirection::Inbound,
        rustls::RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL
        | "SSL_read"
        | "SSL_read_ex"
        | "SSL_read_internal" => PayloadDirection::Inbound,
        _ => PayloadDirection::Control,
    }
}

fn attach_for_symbol(symbol: &str) -> AttachPoint {
    match direction_for_symbol(symbol) {
        PayloadDirection::Inbound
            if matches!(
                symbol,
                "SSL_read" | "SSL_read_ex" | "SSL_read_internal" | go_tls::READ_SYMBOL
            ) =>
        {
            AttachPoint::Return
        }
        PayloadDirection::Inbound | PayloadDirection::Outbound | PayloadDirection::Control => {
            AttachPoint::Entry
        }
    }
}

fn capture_for_symbol(symbol: &str) -> CaptureStrategy {
    match attach_for_symbol(symbol) {
        AttachPoint::Entry => CaptureStrategy::EntryBuffer,
        AttachPoint::Return => CaptureStrategy::ReturnBufferFromEntryState,
    }
}

fn target_identity(image: &ElfImage) -> TargetIdentity {
    TargetIdentity {
        binary: image.path().to_path_buf(),
        architecture: image.arch().as_str().to_string(),
        build_id: image.build_id().map(ToString::to_string),
    }
}

fn probe_binary(image: &ElfImage) -> ProbeBinary {
    ProbeBinary {
        path: image.path().to_path_buf(),
        architecture: image.arch().as_str().to_string(),
        build_id: image.build_id().map(ToString::to_string),
    }
}

fn require_arch(actual: Arch, requested: ArchFilter, path: &std::path::Path) -> ToolResult<()> {
    let matches = match requested {
        ArchFilter::Auto => true,
        ArchFilter::Aarch64 => actual == Arch::Aarch64,
        ArchFilter::X86_64 => actual == Arch::X86_64,
    };
    if matches {
        Ok(())
    } else {
        Err(ToolError::new(format!(
            "{} is {}, not {}",
            path.display(),
            actual.as_str(),
            requested.as_str()
        )))
    }
}

impl ArchFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }
}

impl ProviderFilter {
    fn allows(self, provider: TlsProvider) -> bool {
        match self {
            Self::Auto => true,
            Self::OpenSsl => provider == TlsProvider::OpenSsl,
            Self::BoringSsl => provider == TlsProvider::BoringSsl,
            Self::Rustls => provider == TlsProvider::Rustls,
            Self::Go => provider == TlsProvider::Go,
        }
    }
}

impl SourceFilter {
    fn allows_executable(self) -> bool {
        matches!(self, Self::Auto | Self::Executable)
    }

    fn allows_shared_library(self) -> bool {
        matches!(self, Self::Auto | Self::SharedLibrary)
    }
}

trait WithTarget {
    fn with_target(self, target: &ElfImage) -> Self;
}

impl WithTarget for ProbePointPlan {
    fn with_target(mut self, target: &ElfImage) -> Self {
        self.target = target_identity(target);
        self
    }
}
