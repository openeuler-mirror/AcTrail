//! Dynamic attachment of a detector-produced TLS probe plan.

use std::ffi::OsStr;
use std::path::PathBuf;

use libbpf_rs::{Link, Object, UprobeOpts};
use model_core::binary_identity::BinaryIdentity;

use crate::loader::LoaderError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicTlsProbePlan {
    pub target: PathBuf,
    pub target_identity: BinaryIdentity,
    pub binary: PathBuf,
    pub binary_identity: BinaryIdentity,
    pub provider: String,
    pub points: String,
}

struct DynamicAttachPoint {
    program: &'static str,
    symbol: String,
    offset: u64,
    retprobe: bool,
}

pub(in crate::loader) fn attach_programs(
    object: &mut Object,
    plan: &DynamicTlsProbePlan,
) -> Result<Vec<(Link, String)>, LoaderError> {
    validate_identity(&plan.target, &plan.target_identity, "target")?;
    validate_identity(&plan.binary, &plan.binary_identity, "probe binary")?;
    let points = match plan.provider.as_str() {
        "openssl" => parse_openssl_points(&plan.points)?,
        "rustls" => parse_rustls_points(&plan.points)?,
        provider => {
            return Err(LoaderError::new(
                "attach_dynamic_tls",
                format!("provider {provider} is not supported for direct detector-plan attachment"),
            ));
        }
    };
    let mut links = Vec::with_capacity(points.len());
    for point in points {
        let program = object
            .progs_mut()
            .find(|program| program.name() == OsStr::new(point.program))
            .ok_or_else(|| {
                LoaderError::new(
                    "attach_dynamic_tls",
                    format!("BPF program {} is missing", point.program),
                )
            })?;
        let link = program
            .attach_uprobe_with_opts(
                -1,
                &plan.binary,
                usize::try_from(point.offset).map_err(|_| {
                    LoaderError::new(
                        "attach_dynamic_tls_plan",
                        format!(
                            "probe offset {} does not fit this architecture",
                            point.offset
                        ),
                    )
                })?,
                UprobeOpts {
                    retprobe: point.retprobe,
                    ..Default::default()
                },
            )
            .map_err(|error| {
                LoaderError::new(
                    "attach_dynamic_tls",
                    format!(
                        "attach {} at {}+{:#x}: {error}",
                        point.program,
                        plan.binary.display(),
                        point.offset
                    ),
                )
            })?;
        links.push((
            link,
            format!(
                "{}:{}:{}+{:#x}",
                point.program,
                point.symbol,
                plan.binary.display(),
                point.offset
            ),
        ));
    }
    Ok(links)
}

fn validate_identity(
    path: &std::path::Path,
    expected: &BinaryIdentity,
    label: &str,
) -> Result<(), LoaderError> {
    let actual = tls_probe_point_finder::elf_identity(path).map_err(|error| {
        LoaderError::new(
            "attach_dynamic_tls_identity",
            format!("read {label} identity for {}: {error}", path.display()),
        )
    })?;
    if &actual != expected {
        return Err(LoaderError::new(
            "attach_dynamic_tls_identity",
            format!(
                "{label} identity changed before attachment for {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn parse_rustls_points(value: &str) -> Result<Vec<DynamicAttachPoint>, LoaderError> {
    let mut points = Vec::new();
    let mut has_outbound = false;
    let mut has_inbound = false;
    for encoded in value.split(';').filter(|point| !point.is_empty()) {
        let mut fields = encoded.split(':');
        let symbol = fields.next().unwrap_or_default();
        let direction = fields.next().unwrap_or_default();
        let offset = fields
            .next()
            .ok_or_else(|| invalid_point(encoded, "missing offset"))?
            .parse::<u64>()
            .map_err(|error| invalid_point(encoded, &format!("invalid offset: {error}")))?;
        if fields.next().is_some() {
            return Err(invalid_point(encoded, "unexpected field"));
        }
        let program = match (symbol, direction) {
            ("rustls_buffer_plaintext", "outbound") => {
                has_outbound = true;
                "handle_rustls_buffer_plaintext"
            }
            ("rustls_take_received_plaintext", "inbound") => {
                has_inbound = true;
                "handle_rustls_take_received_plaintext"
            }
            _ => {
                return Err(invalid_point(
                    encoded,
                    "unsupported rustls symbol or direction",
                ));
            }
        };
        points.push(DynamicAttachPoint {
            program,
            symbol: symbol.to_string(),
            offset,
            retprobe: false,
        });
    }
    if !has_outbound || !has_inbound {
        return Err(LoaderError::new(
            "attach_dynamic_tls_plan",
            "rustls direct probe plan must contain one outbound and one inbound point",
        ));
    }
    Ok(points)
}

fn parse_openssl_points(value: &str) -> Result<Vec<DynamicAttachPoint>, LoaderError> {
    let mut points = Vec::new();
    let mut has_outbound = false;
    let mut has_inbound = false;
    for encoded in value.split(';').filter(|point| !point.is_empty()) {
        let mut fields = encoded.split(':');
        let symbol = fields.next().unwrap_or_default();
        let direction = fields.next().unwrap_or_default();
        let offset = fields
            .next()
            .ok_or_else(|| invalid_point(encoded, "missing offset"))?
            .parse::<u64>()
            .map_err(|error| invalid_point(encoded, &format!("invalid offset: {error}")))?;
        if fields.next().is_some() {
            return Err(invalid_point(encoded, "unexpected field"));
        }
        let targets: &[(&str, bool)] = match (symbol, direction) {
            ("SSL_write", "outbound") => {
                has_outbound = true;
                &[
                    ("handle_ssl_write_enter", false),
                    ("handle_ssl_write_exit", true),
                ]
            }
            ("SSL_write_ex", "outbound") => {
                has_outbound = true;
                &[
                    ("handle_ssl_write_ex_enter", false),
                    ("handle_ssl_write_ex_exit", true),
                ]
            }
            ("SSL_read", "inbound") => {
                has_inbound = true;
                &[
                    ("handle_ssl_read_enter", false),
                    ("handle_ssl_read_exit", true),
                ]
            }
            ("SSL_read_ex", "inbound") => {
                has_inbound = true;
                &[
                    ("handle_ssl_read_ex_enter", false),
                    ("handle_ssl_read_ex_exit", true),
                ]
            }
            _ => {
                return Err(invalid_point(
                    encoded,
                    "unsupported OpenSSL symbol or direction",
                ));
            }
        };
        points.extend(
            targets
                .iter()
                .map(|(program, retprobe)| DynamicAttachPoint {
                    program,
                    symbol: symbol.to_string(),
                    offset,
                    retprobe: *retprobe,
                }),
        );
    }
    if !has_outbound || !has_inbound {
        return Err(LoaderError::new(
            "attach_dynamic_tls_plan",
            "OpenSSL direct probe plan must contain an outbound and an inbound point",
        ));
    }
    Ok(points)
}

fn invalid_point(point: &str, reason: &str) -> LoaderError {
    LoaderError::new(
        "attach_dynamic_tls_plan",
        format!("invalid detector point {point:?}: {reason}"),
    )
}
