//! Dynamic Go crypto/tls uprobe attachment.

use std::ffi::OsStr;
use std::path::Path;

use libbpf_rs::{Link, Object, UprobeOpts};

use crate::loader::LoaderError;

use super::targets::GO_UPROBE_TARGETS;
use super::{TlsAttachLocation, go, offset_attach_points, target_symbols};

#[derive(Debug)]
pub(in crate::loader) enum GoTlsAttachOutcome {
    Attached(Vec<(Link, String)>),
    Unsupported,
}

pub(in crate::loader) fn attach_programs(
    object: &mut Object,
    binary_path: &Path,
) -> Result<GoTlsAttachOutcome, LoaderError> {
    let offsets = match go::resolve_offsets(binary_path, &target_symbols(GO_UPROBE_TARGETS)) {
        Ok(offsets) => offsets,
        Err(_) => return Ok(GoTlsAttachOutcome::Unsupported),
    };
    let attach_points =
        offset_attach_points(binary_path, &offsets, GO_UPROBE_TARGETS, "Go crypto/tls")?;
    let mut links = Vec::new();
    for target in attach_points {
        let program = object
            .progs_mut()
            .find(|program| program.name() == OsStr::new(target.program))
            .ok_or_else(|| {
                LoaderError::new(
                    "attach_go_tls",
                    format!("BPF program {} is missing", target.program),
                )
            })?;
        let TlsAttachLocation::Offset { path, offset } = &target.location;
        let link = program
            .attach_uprobe_with_opts(
                -1,
                path,
                *offset,
                UprobeOpts {
                    retprobe: target.retprobe,
                    ..Default::default()
                },
            )
            .map_err(|error| {
                LoaderError::new(
                    "attach_go_tls",
                    format!("attach {} to {}: {error}", target.program, target.label),
                )
            })?;
        links.push((link, format!("{}:{}", target.program, target.label)));
    }
    Ok(GoTlsAttachOutcome::Attached(links))
}
