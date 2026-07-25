use sha2::{Digest, Sha256};

use crate::{ToolError, ToolResult};

use super::{BinaryIdentity, BinaryIdentityTypeCode};

const SAMPLE_WINDOW_BYTES: u64 = 4096;
const SAMPLE_DOMAIN: &[u8] = b"actrail:elf-executable-sample-sha256:v1\0";

#[derive(Clone, Copy)]
pub struct BinaryIdentityRegion {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
}

pub struct BinaryIdentityResolver;

impl BinaryIdentityResolver {
    pub fn resolve(
        build_id: Option<&str>,
        data: &[u8],
        machine: u16,
        executable_regions: &[BinaryIdentityRegion],
    ) -> ToolResult<BinaryIdentity> {
        if let Some(build_id) = build_id {
            return Ok(BinaryIdentity::try_new(
                BinaryIdentityTypeCode::GnuBuildId,
                build_id,
            )?);
        }
        Self::resolve_executable_sample(data, machine, executable_regions)
    }

    fn resolve_executable_sample(
        data: &[u8],
        machine: u16,
        executable_regions: &[BinaryIdentityRegion],
    ) -> ToolResult<BinaryIdentity> {
        if executable_regions.is_empty() {
            return Err(ToolError::new(
                "target has neither a GNU build-id nor an executable ELF LOAD segment",
            ));
        }
        let mut hasher = Sha256::new();
        hasher.update(SAMPLE_DOMAIN);
        hasher.update(machine.to_le_bytes());
        hasher.update((data.len() as u64).to_le_bytes());
        hasher.update((executable_regions.len() as u64).to_le_bytes());
        for region in executable_regions {
            hasher.update(region.file_offset.to_le_bytes());
            hasher.update(region.virtual_address.to_le_bytes());
            hasher.update(region.file_size.to_le_bytes());
            Self::hash_region_samples(&mut hasher, data, *region)?;
        }
        Ok(BinaryIdentity::try_new(
            BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
            format!("{:x}", hasher.finalize()),
        )?)
    }

    fn hash_region_samples(
        hasher: &mut Sha256,
        data: &[u8],
        region: BinaryIdentityRegion,
    ) -> ToolResult<()> {
        let window = region.file_size.min(SAMPLE_WINDOW_BYTES);
        let mut starts = [
            0,
            region.file_size.saturating_sub(window) / 2,
            region.file_size.saturating_sub(window),
        ];
        starts.sort_unstable();
        let mut previous = None;
        for relative in starts {
            if previous == Some(relative) {
                continue;
            }
            previous = Some(relative);
            let start = region
                .file_offset
                .checked_add(relative)
                .ok_or_else(|| ToolError::new("binary identity sample offset overflow"))?;
            let end = start
                .checked_add(window)
                .ok_or_else(|| ToolError::new("binary identity sample end overflow"))?;
            let sample = data
                .get(
                    usize::try_from(start)
                        .map_err(|_| ToolError::new("binary identity sample offset is too large"))?
                        ..usize::try_from(end).map_err(|_| {
                            ToolError::new("binary identity sample end is too large")
                        })?,
                )
                .ok_or_else(|| ToolError::new("binary identity sample exceeds ELF file"))?;
            hasher.update(relative.to_le_bytes());
            hasher.update(window.to_le_bytes());
            hasher.update(sample);
        }
        Ok(())
    }
}
