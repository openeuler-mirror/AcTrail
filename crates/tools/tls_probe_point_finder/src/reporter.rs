//! Formatting for human-readable probe-point reports.

use std::fmt::{Display, Write as FmtWrite};
use std::io::{self, Write as IoWrite};

use anstyle::{AnsiColor, Style};

use crate::ToolResult;
use crate::detect::{
    CandidateReport, CandidateStatus, DetectReport, ExportedSymbolReport, MapStatusReport,
    PatternMatchesReport, SymbolMapReport, TargetReport,
};
use crate::pattern_cmd::PatternReport;
use crate::plan::ProbePointPlan;

// Indentation is intentionally controlled only here; docs record the visible
// format as two spaces per nesting level.
const INDENT_UNIT: &str = "  ";

pub(crate) fn print_detect_report(report: &DetectReport) -> ToolResult<()> {
    let mut reporter = Reporter::new();
    reporter.target(&report.target);
    if !report.notices.is_empty() {
        reporter.block("notices", |reporter| {
            for notice in &report.notices {
                reporter.list_value(notice);
            }
        });
    }
    if !report.candidates.is_empty() {
        reporter.block("candidates", |reporter| {
            for candidate in &report.candidates {
                reporter.candidate(candidate);
            }
        });
    }
    reporter.flush()
}

pub(crate) fn print_pattern_report(report: &PatternReport) -> ToolResult<()> {
    let mut reporter = Reporter::new();
    reporter.target(&report.target);
    reporter.block("pattern", |reporter| {
        reporter.field("address", &report.address);
        reporter.field("file_offset", &report.file_offset);
        reporter.field("length", &report.length);
        reporter.field("match_count", report.match_count);
        reporter.field("pattern_hex", &report.pattern_hex);
        reporter.block("matches", |reporter| {
            for found in &report.matches {
                reporter.list_field_block("file_offset", &found.file_offset, |reporter| {
                    reporter.field("virtual_address", &found.virtual_address);
                });
            }
        });
    });
    reporter.flush()
}

pub(crate) fn print_fast_probe_plan(plan: &ProbePointPlan) -> ToolResult<()> {
    let mut reporter = Reporter::new();
    reporter.block("target", |reporter| {
        reporter.field("binary", plan.target.binary.display());
        reporter.field("architecture", &plan.target.architecture);
        reporter.field(
            "build_id",
            plan.target.build_id.as_deref().unwrap_or("not_found"),
        );
    });
    reporter.block("probe_plan", |reporter| {
        reporter.field("provider", plan.provider.as_str());
        reporter.field("source", plan.source.as_str());
        reporter.field("resolver", &plan.resolver);
        reporter.block("binary", |reporter| {
            reporter.field("path", plan.binary.path.display());
            reporter.field("architecture", &plan.binary.architecture);
            reporter.field(
                "build_id",
                plan.binary.build_id.as_deref().unwrap_or("not_found"),
            );
        });
        reporter.block("points", |reporter| {
            for point in &plan.points {
                reporter.list_field_block("symbol", &point.symbol, |reporter| {
                    reporter.field("direction", point.direction.as_str());
                    reporter.field("attach", point.attach.as_str());
                    reporter.field("capture", point.capture.as_str());
                    reporter.field(
                        "virtual_address",
                        format_args!("0x{:x}", point.virtual_address),
                    );
                    reporter.field("file_offset", format_args!("0x{:x}", point.file_offset));
                });
            }
        });
    });
    reporter.flush()
}

struct Reporter {
    buffer: String,
    level: usize,
}

impl Reporter {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            level: 0,
        }
    }

    fn target(&mut self, target: &TargetReport) {
        self.block("target", |reporter| {
            reporter.field("binary", &target.binary);
            reporter.field("architecture", &target.architecture);
            reporter.field("build_id", &target.build_id);
        });
    }

    fn candidate(&mut self, candidate: &CandidateReport) {
        self.candidate_item(candidate, |reporter| {
            if let Some(library) = &candidate.library {
                reporter.block("library", |reporter| {
                    reporter.field("path", &library.path);
                    reporter.field("confidence", &library.confidence);
                    if let Some(note) = &library.note {
                        reporter.field("note", note);
                    }
                    if let Some(architecture) = &library.architecture {
                        reporter.field("architecture", architecture);
                    }
                    if let Some(build_id) = &library.build_id {
                        reporter.field("build_id", build_id);
                    }
                });
            }
            if !candidate.exported_symbols.is_empty() {
                reporter.exported_symbols(&candidate.exported_symbols);
            }
            if let Some(status) = &candidate.exported_symbol_map {
                reporter.map_status("exported_symbol_map", status);
            }
            if let Some(demangled) = &candidate.demangled_symbols {
                reporter.block("demangled_rust_symbols", |reporter| {
                    reporter.field("status", &demangled.status);
                    reporter.field("source", &demangled.source);
                    reporter.field("binary", &demangled.binary);
                    reporter.block("targets", |reporter| {
                        for target in &demangled.targets {
                            reporter.list_field_block("symbol", &target.symbol, |reporter| {
                                reporter.field("address", &target.address);
                                reporter.field("runtime_symbol", &target.runtime_symbol);
                            });
                        }
                    });
                });
            }
            if let Some(status) = &candidate.endpoint_status {
                reporter.map_status("endpoints", status);
            }
            if !candidate.endpoints.is_empty() {
                reporter.block("endpoints", |reporter| {
                    for endpoint in &candidate.endpoints {
                        reporter.list_field_block("symbol", &endpoint.symbol, |reporter| {
                            reporter.field("virtual_address", &endpoint.virtual_address);
                            reporter.field("file_offset", &endpoint.file_offset);
                        });
                    }
                });
            }
            if let Some(patterns) = &candidate.pattern_matches {
                reporter.pattern_matches(patterns);
            }
            if !candidate.detected_offsets.is_empty() {
                reporter.block("detected_offsets", |reporter| {
                    for offset in &candidate.detected_offsets {
                        reporter.list_field_block("symbol", &offset.symbol, |reporter| {
                            reporter.field("file_offset", &offset.file_offset);
                            reporter.field("virtual_address", &offset.virtual_address);
                        });
                    }
                });
            }
            if !candidate.runtime_config.is_empty() {
                reporter.block("runtime_config", |reporter| {
                    for (key, value) in &candidate.runtime_config {
                        reporter.field(key, value);
                    }
                });
            }
            if let Some(symbol_map) = &candidate.symbol_map {
                reporter.symbol_map(symbol_map);
            }
            match &candidate.status {
                CandidateStatus::Matched => reporter.field("status", "matched"),
                CandidateStatus::Available => reporter.field("status", "available"),
                CandidateStatus::Failed { error } => {
                    reporter.field("status", "failed");
                    reporter.field("error", error);
                }
            }
        });
    }

    fn exported_symbols(&mut self, symbols: &[ExportedSymbolReport]) {
        self.block("exported_symbols", |reporter| {
            for symbol in symbols {
                if symbol.entries.is_empty() {
                    reporter.symbol_status_item(&symbol.name, "not_found");
                    continue;
                }
                reporter.symbol_item(&symbol.name, |reporter| {
                    reporter.block("entries", |reporter| {
                        for entry in &symbol.entries {
                            reporter.list_field_block("value", &entry.value, |reporter| {
                                reporter.field("size", &entry.size);
                                reporter.field("bind", &entry.bind);
                                reporter.field("ndx", &entry.ndx);
                                reporter.field("table", &entry.table);
                                reporter.field("raw", &entry.raw);
                            });
                        }
                    });
                });
            }
        });
    }

    fn pattern_matches(&mut self, patterns: &PatternMatchesReport) {
        self.block("pattern_matches", |reporter| {
            reporter.field("arch", &patterns.arch);
            for entry in &patterns.entries {
                reporter.list_labeled_block(&entry.pattern_id, |reporter| {
                    reporter.field("symbol", &entry.symbol);
                    reporter.field("library", &entry.library);
                    reporter.field("resolver", &entry.resolver);
                    reporter.field("pattern_length", &entry.pattern_length);
                    reporter.field("match_count", entry.match_count);
                    reporter.block("matches", |reporter| {
                        for found in &entry.matches {
                            reporter.list_field_block(
                                "file_offset",
                                &found.file_offset,
                                |reporter| {
                                    reporter.field("virtual_address", &found.virtual_address);
                                },
                            );
                        }
                    });
                });
            }
        });
    }

    fn symbol_map(&mut self, symbol_map: &SymbolMapReport) {
        self.block("symbol_map", |reporter| {
            reporter.field("resolver", &symbol_map.resolver);
            reporter.field("library", &symbol_map.library);
            reporter.field("arch", &symbol_map.arch);
            reporter.field("build_id", &symbol_map.build_id);
            if !symbol_map.symbols.is_empty() {
                reporter.block("symbols", |reporter| {
                    for (symbol, address) in &symbol_map.symbols {
                        reporter.symbol_map_item(symbol, address);
                    }
                });
            }
        });
    }

    fn map_status(&mut self, label: &str, status: &MapStatusReport) {
        self.block(label, |reporter| {
            reporter.field("status", &status.status);
            for (key, value) in &status.fields {
                if key == "missing" {
                    reporter.missing_symbols(value);
                } else {
                    reporter.field(key, value);
                }
            }
        });
    }

    fn block(&mut self, label: &str, write: impl FnOnce(&mut Self)) {
        self.line(format_args!("{label}:"));
        self.level += 1;
        write(self);
        self.level -= 1;
    }

    fn candidate_item(&mut self, candidate: &CandidateReport, write: impl FnOnce(&mut Self)) {
        let source_style = source_style();
        let provider_style = provider_style();
        self.line(format_args!(
            "- {source_style}{}{source_style:#}/{provider_style}{}{provider_style:#}",
            candidate.source, candidate.provider
        ));
        self.level += 1;
        write(self);
        self.level -= 1;
    }

    fn list_labeled_block(&mut self, label: impl Display, write: impl FnOnce(&mut Self)) {
        self.line(format_args!("- {label}"));
        self.level += 1;
        write(self);
        self.level -= 1;
    }

    fn list_value(&mut self, value: impl Display) {
        self.line(format_args!("- {value}"));
    }

    fn field(&mut self, key: &str, value: impl Display) {
        if let Some(style) = field_style(key) {
            self.line(format_args!("{key} = {style}{value}{style:#}"));
        } else {
            self.line(format_args!("{key} = {value}"));
        }
    }

    fn list_field_block(&mut self, key: &str, value: impl Display, write: impl FnOnce(&mut Self)) {
        if let Some(style) = field_style(key) {
            self.line(format_args!("- {key} = {style}{value}{style:#}"));
        } else {
            self.line(format_args!("- {key} = {value}"));
        }
        self.level += 1;
        write(self);
        self.level -= 1;
    }

    fn symbol_item(&mut self, symbol: &str, write: impl FnOnce(&mut Self)) {
        let style = symbol_style();
        self.line(format_args!("- {style}{symbol}{style:#}:"));
        self.level += 1;
        write(self);
        self.level -= 1;
    }

    fn symbol_status_item(&mut self, symbol: &str, status: &str) {
        let symbol_style = symbol_style();
        let status_style = missing_style();
        self.line(format_args!(
            "- {symbol_style}{symbol}{symbol_style:#}: {status_style}{status}{status_style:#}"
        ));
    }

    fn symbol_map_item(&mut self, symbol: &str, address: &str) {
        let symbol_style = symbol_style();
        let address_style = address_style();
        self.line(format_args!(
            "- {symbol_style}{symbol}{symbol_style:#}|{address_style}{address}{address_style:#}"
        ));
    }

    fn missing_symbols(&mut self, symbols: &str) {
        let style = symbol_style();
        self.block("missing", |reporter| {
            for symbol in symbols.split(',') {
                reporter.line(format_args!("- {style}{symbol}{style:#}"));
            }
        });
    }

    fn line(&mut self, args: std::fmt::Arguments<'_>) {
        for _ in 0..self.level {
            self.buffer.push_str(INDENT_UNIT);
        }
        let _ = self.buffer.write_fmt(args);
        self.buffer.push('\n');
    }

    fn flush(&mut self) -> ToolResult<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(self.buffer.as_bytes())?;
        stdout.flush()?;
        self.buffer.clear();
        Ok(())
    }
}

fn field_style(key: &str) -> Option<Style> {
    match key {
        "address" | "file_offset" | "value" | "virtual_address" => Some(address_style()),
        "raw" | "runtime_symbol" | "symbol" => Some(symbol_style()),
        _ => None,
    }
}

fn symbol_style() -> Style {
    Style::new().bold().fg_color(Some(AnsiColor::Cyan.into()))
}

fn missing_style() -> Style {
    Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()))
}

fn address_style() -> Style {
    Style::new().bold().fg_color(Some(AnsiColor::Green.into()))
}

fn source_style() -> Style {
    Style::new().bold().fg_color(Some(AnsiColor::Blue.into()))
}

fn provider_style() -> Style {
    Style::new()
        .bold()
        .fg_color(Some(AnsiColor::Magenta.into()))
}
