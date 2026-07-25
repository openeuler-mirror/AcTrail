use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::probe_detector::contract::selection::SelectionPolicy;

use super::boringssl::BoringSslProbeDetectorConfig;
use super::gnutls::GnuTlsProbeDetectorConfig;
use super::go_tls::GoTlsProbeDetectorConfig;
use super::nss::NssProbeDetectorConfig;
use super::openssl::OpenSslProbeDetectorConfig;
use super::rustls::RustlsProbeDetectorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TlsProbeDetectorConfig {
    pub(crate) selection: SelectionPolicy,
    pub(crate) rustls: RustlsProbeDetectorConfig,
    pub(crate) boringssl: BoringSslProbeDetectorConfig,
    pub(crate) openssl: OpenSslProbeDetectorConfig,
    pub(crate) go_tls: GoTlsProbeDetectorConfig,
    pub(crate) gnutls: GnuTlsProbeDetectorConfig,
    pub(crate) nss: NssProbeDetectorConfig,
}

impl TlsProbeDetectorConfig {
    pub(crate) fn with_match_limit(match_limit: usize) -> Self {
        Self {
            rustls: RustlsProbeDetectorConfig::with_match_limit(match_limit),
            boringssl: BoringSslProbeDetectorConfig::with_match_limit(match_limit),
            ..Self::default()
        }
    }

    pub(crate) fn for_diagnostics(match_limit: usize) -> Self {
        let mut config = Self::with_match_limit(match_limit);
        config.selection = SelectionPolicy::CollectAll;
        config.rustls.selection = SelectionPolicy::CollectAll;
        config
    }
}

impl Default for TlsProbeDetectorConfig {
    fn default() -> Self {
        Self {
            selection: SelectionPolicy::FirstComplete,
            rustls: RustlsProbeDetectorConfig::default(),
            boringssl: BoringSslProbeDetectorConfig::default(),
            openssl: OpenSslProbeDetectorConfig::default(),
            go_tls: GoTlsProbeDetectorConfig::default(),
            gnutls: GnuTlsProbeDetectorConfig::default(),
            nss: NssProbeDetectorConfig::default(),
        }
    }
}

impl ProbeDetectorConfig for TlsProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        self.rustls.validate()?;
        self.boringssl.validate()?;
        self.openssl.validate()?;
        self.go_tls.validate()?;
        self.gnutls.validate()?;
        self.nss.validate()
    }
}
