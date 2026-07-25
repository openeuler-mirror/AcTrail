use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};

use super::GoTlsProbeDetectorConfig;
use super::pclntab::GoPclntabProbeDetector;

pub(crate) const NAME: &str = "go";
pub(crate) const RESOLVER: &str = "go-pclntab";
pub(crate) const WRITE_SYMBOL: &str = "crypto/tls.(*Conn).Write";
pub(crate) const READ_SYMBOL: &str = "crypto/tls.(*Conn).Read";
pub(crate) const RUNTIME_MEMMOVE_SYMBOL: &str = "runtime.memmove";
pub(crate) const SYMBOLS: &[&str] = &[WRITE_SYMBOL, READ_SYMBOL, RUNTIME_MEMMOVE_SYMBOL];

pub(crate) struct GoTlsProbeDetector {
    path: DetectorPath,
    pclntab: GoPclntabProbeDetector,
}

impl GoTlsProbeDetector {
    pub(crate) fn try_new(config: GoTlsProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id.clone()),
            pclntab: GoPclntabProbeDetector::try_new(config.pclntab)?,
        })
    }

    pub(crate) fn resolve(
        &self,
        image: &ElfImage,
        required_symbols: &[&str],
    ) -> ToolResult<Option<BTreeMap<String, u64>>> {
        self.pclntab.resolve(image, required_symbols)
    }
}

impl ProbeDetector for GoTlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.request.requested_source == Some(ProbeSource::SharedLibrary)
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Go)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Go crypto/tls executable detector excluded by request"),
            ));
        }
        self.pclntab.detect(context)
    }
}
