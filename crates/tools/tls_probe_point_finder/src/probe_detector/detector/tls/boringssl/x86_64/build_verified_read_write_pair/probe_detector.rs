use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::common::{
    StaticPatternDetection, StaticPatternSupport,
};
use crate::probe_detector::detector::tls::boringssl::x86_64::static_pattern::X86_64BoringSslStaticPatternProbeDetector;
use crate::{BinaryIdentityTypeCode, ToolError, ToolResult};

use super::BuildVerifiedReadWritePairDetectorConfig;
use super::specifications::{SPECIFICATIONS, VerifiedReadWritePairSpec};

pub(crate) struct BuildVerifiedReadWritePairDetector {
    specification: &'static VerifiedReadWritePairSpec,
    match_limit: usize,
}

impl BuildVerifiedReadWritePairDetector {
    pub(crate) fn try_from_specifications(
        config: BuildVerifiedReadWritePairDetectorConfig,
    ) -> Result<[Self; 2], DetectorConfigError> {
        config.validate()?;
        Ok(SPECIFICATIONS.each_ref().map(|specification| Self {
            specification,
            match_limit: config.match_limit,
        }))
    }

    pub(crate) fn detector_id(&self) -> &'static str {
        self.specification.detector_id
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        if !self.matches_build(image) {
            return;
        }
        image.register_pattern_scan(self.specification.read_pattern);
        image.register_pattern_scan(self.specification.write_pattern);
    }

    pub(crate) fn detect(&self, image: &ElfImage) -> ToolResult<Option<StaticPatternDetection>> {
        if !self.matches_build(image) {
            return Ok(None);
        }
        let spec = self.specification;
        let read_matches = StaticPatternSupport::find_all_executable(image, spec.read_pattern)?;
        let read = StaticPatternSupport::require_single(&read_matches, "SSL_read")?;
        let write_matches = StaticPatternSupport::find_all_executable(image, spec.write_pattern)?;
        let write = StaticPatternSupport::require_single(&write_matches, "SSL_write")?;
        if !X86_64BoringSslStaticPatternProbeDetector::has_identity(image) {
            return Err(ToolError::new("BoringSSL identity markers were not found"));
        }
        Ok(Some(StaticPatternDetection {
            arch_label: "x86_64",
            matches: vec![
                StaticPatternSupport::pattern_matches(
                    image,
                    spec.read_pattern_id,
                    "SSL_read",
                    spec.read_pattern,
                    &read_matches,
                    self.match_limit,
                )?,
                StaticPatternSupport::pattern_matches(
                    image,
                    spec.write_pattern_id,
                    "SSL_write",
                    spec.write_pattern,
                    &write_matches,
                    self.match_limit,
                )?,
            ],
            offsets: StaticPatternSupport::offsets_with_addresses(
                image,
                &[("SSL_read", read), ("SSL_write", write)],
            )?,
        }))
    }

    fn matches_build(&self, image: &ElfImage) -> bool {
        let identity = image.identity();
        identity.identity_type_code == BinaryIdentityTypeCode::GnuBuildId
            && identity.identity == self.specification.build_id
    }
}
