//! TLS provider detection command.

use crate::ToolResult;
use crate::args::{DetectArgs, ProviderChoice, SourceChoice, require_arch};
use crate::binary::resolve_entry_elf;
use crate::elf::ElfImage;
use crate::providers::{boringssl, openssl, rustls};

use super::assemble::{
    detected_offsets_report, exported_symbols, missing_symbols, names_with_extra,
    pattern_matches_report, rustls_detected_offsets_report, rustls_pattern_matches_report,
    symbol_map_report,
};
use super::report::*;

pub(crate) fn run(args: DetectArgs) -> ToolResult<DetectReport> {
    let binary = resolve_entry_elf(&args.binary)?;
    let image = ElfImage::parse(&binary)?;
    require_arch(image.arch(), args.arch, image.path())?;
    let mut report = DetectReport {
        target: TargetReport::from_image(&image),
        notices: Vec::new(),
        candidates: Vec::new(),
    };

    if matches!(args.source, SourceChoice::Auto | SourceChoice::Executable) {
        for provider in providers(args.provider) {
            let explicit = args.provider != ProviderChoice::Auto;
            let candidate = detect_executable_provider(&image, provider, &args, explicit)
                .unwrap_or_else(|error| {
                    CandidateReport::failed(
                        "executable",
                        provider_name(provider),
                        error.to_string(),
                    )
                });
            report.candidates.push(candidate);
        }
    }
    if matches!(
        args.source,
        SourceChoice::Auto | SourceChoice::SharedLibrary
    ) {
        let (mut candidates, notices) = detect_shared_libraries(&image, &args);
        report.notices.extend(notices);
        report.candidates.append(&mut candidates);
    }
    Ok(report)
}

fn detect_executable_provider(
    image: &ElfImage,
    provider: ProviderChoice,
    args: &DetectArgs,
    explicit: bool,
) -> ToolResult<CandidateReport> {
    match provider {
        ProviderChoice::OpenSsl => detect_executable_openssl(image, args),
        ProviderChoice::BoringSsl => detect_executable_boringssl(image, args, explicit),
        ProviderChoice::Rustls => detect_executable_rustls(image, args),
        ProviderChoice::Auto => unreachable!("auto provider is expanded before detection"),
    }
}

fn detect_executable_openssl(image: &ElfImage, args: &DetectArgs) -> ToolResult<CandidateReport> {
    let mut candidate = CandidateReport::new("executable", openssl::NAME);
    candidate.exported_symbols =
        exported_symbols(image, &names_with_extra(openssl::SYMBOLS, &args.symbols))?;
    let symbols = image.unique_defined_symbol_values(openssl::SYMBOLS)?;
    let missing = missing_symbols(openssl::SYMBOLS, &symbols);
    if !missing.is_empty() {
        candidate.exported_symbol_map = Some(MapStatusReport::missing("incomplete", &missing));
        candidate.status = CandidateStatus::Failed {
            error:
                "OpenSSL provider requires exported SSL_read/write and SSL_read_ex/write_ex symbols"
                    .to_string(),
        };
        return Ok(candidate);
    }
    candidate.symbol_map = Some(symbol_map_report(
        openssl::RESOLVER,
        openssl::LIBRARY,
        image.arch(),
        image.build_id(),
        &symbols,
    )?);
    candidate.status = CandidateStatus::Matched;
    Ok(candidate)
}

fn detect_executable_boringssl(
    image: &ElfImage,
    args: &DetectArgs,
    explicit: bool,
) -> ToolResult<CandidateReport> {
    let mut candidate = CandidateReport::new("executable", boringssl::NAME);
    candidate.exported_symbols =
        exported_symbols(image, &names_with_extra(boringssl::SYMBOLS, &args.symbols))?;
    let map_symbols = boringssl::map_symbols(image.arch());
    let symbols = image.unique_defined_symbol_values(map_symbols)?;
    let missing = missing_symbols(map_symbols, &symbols);
    if missing.is_empty() && explicit {
        candidate.symbol_map = Some(symbol_map_report(
            boringssl::SYMBOL_MAP_RESOLVER,
            boringssl::LIBRARY,
            image.arch(),
            image.build_id(),
            &symbols,
        )?);
        candidate.status = CandidateStatus::Matched;
        return Ok(candidate);
    }
    candidate.exported_symbol_map = if missing.is_empty() {
        Some(MapStatusReport {
            status: "skipped".to_string(),
            fields: vec![(
                "reason".to_string(),
                "shared SSL_* names do not prove BoringSSL in auto mode".to_string(),
            )],
        })
    } else {
        Some(MapStatusReport::missing("incomplete", &missing))
    };
    match boringssl::detect_static_patterns(image, args.match_limit) {
        Ok(detection) => {
            candidate.pattern_matches = Some(pattern_matches_report(&detection));
            candidate.detected_offsets = detected_offsets_report(&detection);
            candidate.runtime_config = vec![
                ("payload_tls_source".to_string(), "executable".to_string()),
                (
                    "payload_tls_resolver".to_string(),
                    boringssl::STATIC_RESOLVER.to_string(),
                ),
                (
                    "payload_tls_library".to_string(),
                    boringssl::LIBRARY.to_string(),
                ),
                (
                    "payload_tls_binary_path".to_string(),
                    image.path().display().to_string(),
                ),
                (
                    "payload_tls_pattern_path".to_string(),
                    "disabled".to_string(),
                ),
            ];
            candidate.symbol_map = Some(symbol_map_report(
                boringssl::SYMBOL_MAP_RESOLVER,
                boringssl::LIBRARY,
                image.arch(),
                image.build_id(),
                &detection.map_symbols,
            )?);
            candidate.status = CandidateStatus::Matched;
        }
        Err(error) => {
            candidate.status = CandidateStatus::Failed {
                error: error.to_string(),
            };
        }
    }
    Ok(candidate)
}

fn detect_executable_rustls(image: &ElfImage, args: &DetectArgs) -> ToolResult<CandidateReport> {
    let mut candidate = CandidateReport::new("executable", rustls::NAME);
    if !args.symbols.is_empty() {
        candidate.exported_symbols = exported_symbols(image, &args.symbols)?;
    }
    match rustls::resolve_demangled_plaintext_symbols(image)? {
        Some(symbols) => {
            candidate.demangled_symbols = Some(DemangledSymbolReport {
                status: "matched".to_string(),
                source: "elf-symbol-table".to_string(),
                binary: image.path().display().to_string(),
                targets: symbols
                    .targets
                    .iter()
                    .map(|target| DemangledSymbolTargetReport {
                        symbol: target.symbol.clone(),
                        address: format!("0x{:x}", target.address),
                        runtime_symbol: target.runtime_symbol.to_string(),
                    })
                    .collect(),
            });
            candidate.symbol_map = Some(symbol_map_report(
                rustls::RESOLVER,
                rustls::LIBRARY,
                image.arch(),
                image.build_id(),
                &symbols.runtime_symbols,
            )?);
            candidate.status = CandidateStatus::Matched;
        }
        None => match rustls::detect_static_patterns(image, args.match_limit) {
            Ok(detection) => {
                candidate.pattern_matches = Some(rustls_pattern_matches_report(&detection));
                candidate.detected_offsets = rustls_detected_offsets_report(&detection);
                candidate.symbol_map = Some(symbol_map_report(
                    rustls::RESOLVER,
                    rustls::LIBRARY,
                    image.arch(),
                    image.build_id(),
                    &detection.map_symbols,
                )?);
                candidate.status = CandidateStatus::Matched;
            }
            Err(error) => {
                candidate.status = CandidateStatus::Failed {
                    error: error.to_string(),
                };
            }
        },
    }
    Ok(candidate)
}

fn detect_shared_libraries(
    image: &ElfImage,
    args: &DetectArgs,
) -> (Vec<CandidateReport>, Vec<String>) {
    if matches!(
        args.provider,
        ProviderChoice::BoringSsl | ProviderChoice::Rustls
    ) {
        return (
            vec![CandidateReport::failed(
                "shared-library",
                provider_name(args.provider),
                "shared-library detection is only implemented for OpenSSL",
            )],
            Vec::new(),
        );
    }
    let search =
        match openssl::library_candidates(image, &args.libraries, &args.library_search_dirs) {
            Ok(search) => search,
            Err(error) => {
                return (
                    vec![CandidateReport::failed(
                        "shared-library",
                        openssl::NAME,
                        error.to_string(),
                    )],
                    Vec::new(),
                );
            }
        };
    if search.candidates.is_empty() {
        let candidates = if args.source == SourceChoice::SharedLibrary {
            vec![CandidateReport::failed(
                "shared-library",
                openssl::NAME,
                "no libssl shared-library candidates found",
            )]
        } else {
            Vec::new()
        };
        return (candidates, search.notices);
    }
    let candidates = search
        .candidates
        .iter()
        .map(|candidate| detect_openssl_library_candidate(image, candidate, args))
        .collect();
    (candidates, search.notices)
}

fn detect_openssl_library_candidate(
    target: &ElfImage,
    candidate: &openssl::LibraryCandidate,
    args: &DetectArgs,
) -> CandidateReport {
    let mut report = CandidateReport::new("shared-library", openssl::NAME);
    report.library = Some(LibraryReport {
        path: candidate.path.display().to_string(),
        confidence: candidate.confidence.to_string(),
        note: candidate.note.clone(),
        architecture: None,
        build_id: None,
    });
    let library = match ElfImage::parse(&candidate.path) {
        Ok(library) => library,
        Err(error) => {
            report.status = CandidateStatus::Failed {
                error: error.to_string(),
            };
            return report;
        }
    };
    if let Some(library_report) = &mut report.library {
        library_report.architecture = Some(library.arch().as_str().to_string());
        library_report.build_id = Some(library.build_id().unwrap_or("not_found").to_string());
    }
    if library.arch() != target.arch() {
        report.status = CandidateStatus::Failed {
            error: format!(
                "OpenSSL shared library architecture {} does not match target architecture {}",
                library.arch().as_str(),
                target.arch().as_str()
            ),
        };
        return report;
    }
    report.exported_symbols =
        match exported_symbols(&library, &names_with_extra(openssl::SYMBOLS, &args.symbols)) {
            Ok(symbols) => symbols,
            Err(error) => {
                report.status = CandidateStatus::Failed {
                    error: error.to_string(),
                };
                return report;
            }
        };
    let symbols = match library.unique_defined_symbol_values(openssl::SYMBOLS) {
        Ok(symbols) => symbols,
        Err(error) => {
            report.status = CandidateStatus::Failed {
                error: error.to_string(),
            };
            return report;
        }
    };
    let missing = missing_symbols(openssl::SYMBOLS, &symbols);
    if !missing.is_empty() {
        report.endpoint_status = Some(MapStatusReport::missing("incomplete", &missing));
        report.status = CandidateStatus::Failed {
            error: "OpenSSL shared library is missing required endpoints".to_string(),
        };
        return report;
    }
    for symbol in openssl::SYMBOLS {
        let address = symbols
            .get(*symbol)
            .copied()
            .expect("required symbol present");
        let file_offset = match library.file_offset_for_virtual_address(address) {
            Ok(file_offset) => file_offset,
            Err(error) => {
                report.status = CandidateStatus::Failed {
                    error: error.to_string(),
                };
                return report;
            }
        };
        report.endpoints.push(EndpointReport {
            symbol: (*symbol).to_string(),
            virtual_address: format!("0x{address:x}"),
            file_offset: format!("0x{file_offset:x}"),
        });
    }
    report.runtime_config = vec![
        (
            "payload_tls_source".to_string(),
            "shared-library".to_string(),
        ),
        (
            "payload_tls_resolver".to_string(),
            openssl::RESOLVER.to_string(),
        ),
        (
            "payload_tls_library".to_string(),
            openssl::LIBRARY.to_string(),
        ),
        (
            "payload_tls_library_path".to_string(),
            candidate.path.display().to_string(),
        ),
    ];
    report.status = if candidate.counts_as_match || args.source == SourceChoice::SharedLibrary {
        CandidateStatus::Matched
    } else {
        CandidateStatus::Available
    };
    report
}

fn providers(choice: ProviderChoice) -> Vec<ProviderChoice> {
    match choice {
        ProviderChoice::Auto => vec![
            ProviderChoice::OpenSsl,
            ProviderChoice::BoringSsl,
            ProviderChoice::Rustls,
        ],
        provider => vec![provider],
    }
}

fn provider_name(provider: ProviderChoice) -> &'static str {
    match provider {
        ProviderChoice::Auto => "auto",
        ProviderChoice::OpenSsl => openssl::NAME,
        ProviderChoice::BoringSsl => boringssl::NAME,
        ProviderChoice::Rustls => rustls::NAME,
    }
}
