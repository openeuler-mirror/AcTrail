use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::detector::tls::boringssl::MAP_SYMBOLS_AARCH64;
use crate::probe_detector::detector::tls::boringssl::common::BoringSslSymbolEvidence;

use super::Aarch64BoringSslSharedLibraryProbeDetectorConfig;

pub(crate) struct Aarch64BoringSslSharedLibraryProbeDetector {
    evidence: BoringSslSymbolEvidence,
}

impl Aarch64BoringSslSharedLibraryProbeDetector {
    pub(crate) fn try_new(
        config: Aarch64BoringSslSharedLibraryProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            evidence: BoringSslSymbolEvidence::new(MAP_SYMBOLS_AARCH64),
        })
    }

    pub(crate) fn resolve(&self, image: &ElfImage) -> ToolResult<Option<BTreeMap<String, u64>>> {
        self.evidence.resolve(image)
    }
}
