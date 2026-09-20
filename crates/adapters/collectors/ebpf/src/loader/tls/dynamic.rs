//! Attach detector plans after provider-specific translation.

#[path = "dynamic/points.rs"]
mod points;
#[path = "dynamic/rustls.rs"]
mod rustls;
#[path = "dynamic/ssl.rs"]
mod ssl;

use crate::loader::LoaderError;
use libbpf_rs::{Link, Object, UprobeOpts};
use model_core::binary_identity::BinaryIdentity;
use points::AttachmentPlan;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicTlsProbePlan {
    pub target: PathBuf,
    pub target_identity: BinaryIdentity,
    pub binary: PathBuf,
    pub binary_identity: BinaryIdentity,
    pub provider: String,
    pub points: String,
}

impl DynamicTlsProbePlan {
    fn validate_identity(
        path: &Path,
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

    fn attach(&self, object: &mut Object) -> Result<Vec<(Link, String)>, LoaderError> {
        Self::validate_identity(&self.target, &self.target_identity, "target")?;
        Self::validate_identity(&self.binary, &self.binary_identity, "probe binary")?;
        let attachment = AttachmentPlan::resolve(&self.provider, &self.points)?;
        let mut links = Vec::with_capacity(attachment.points.len());
        for point in attachment.points {
            let program = object
                .progs_mut()
                .find(|program| program.name() == OsStr::new(point.program))
                .ok_or_else(|| {
                    LoaderError::new(
                        "attach_dynamic_tls",
                        format!("BPF program {} is missing", point.program),
                    )
                })?;
            let offset = usize::try_from(point.offset).map_err(|_| {
                LoaderError::new(
                    "attach_dynamic_tls_plan",
                    "probe offset exceeds architecture size",
                )
            })?;
            let link = program
                .attach_uprobe_with_opts(
                    -1,
                    &self.binary,
                    offset,
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
                            self.binary.display(),
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
                    self.binary.display(),
                    point.offset
                ),
            ));
        }
        Ok(links)
    }
}

pub(in crate::loader) fn attach_programs(
    object: &mut Object,
    plan: &DynamicTlsProbePlan,
) -> Result<Vec<(Link, String)>, LoaderError> {
    plan.attach(object)
}
