use crate::elf::Arch;
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, EvidenceFact, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::TlsProbeDetectorConfig;
use super::boringssl::BoringSslProbeDetector;
use super::boringssl::{MAP_SYMBOLS_AARCH64, MAP_SYMBOLS_X86_64};
use super::gnutls::GnuTlsProbeDetector;
use super::gnutls::SYMBOLS as GNUTLS_SYMBOLS;
use super::go_tls::GoTlsProbeDetector;
use super::nss::NssProbeDetector;
use super::nss::SYMBOLS as NSS_SYMBOLS;
use super::openssl::OpenSslProbeDetector;
use super::openssl::PROBE_SYMBOLS as OPENSSL_PROBE_SYMBOLS;
use super::rustls::RustlsProbeDetector;
use crate::probe_detector::detector::tls::ExecutablePatternRegistration;

pub(crate) struct TlsProbeDetector {
    path: DetectorPath,
    selector: DetectionSelector,
    rustls: RustlsProbeDetector,
    boringssl: BoringSslProbeDetector,
    openssl: OpenSslProbeDetector,
    go_tls: GoTlsProbeDetector,
    gnutls: GnuTlsProbeDetector,
    nss: NssProbeDetector,
}

impl TlsProbeDetector {
    pub(crate) fn try_new(config: TlsProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new("tls");
        Ok(Self {
            path: DetectorPath::root(id.clone()),
            selector: DetectionSelector::new(config.selection),
            rustls: RustlsProbeDetector::try_new(config.rustls)?,
            boringssl: BoringSslProbeDetector::try_new(config.boringssl)?,
            openssl: OpenSslProbeDetector::try_new(config.openssl)?,
            go_tls: GoTlsProbeDetector::try_new(config.go_tls)?,
            gnutls: GnuTlsProbeDetector::try_new(config.gnutls)?,
            nss: NssProbeDetector::try_new(config.nss)?,
        })
    }
}

impl ProbeDetector for TlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        // 符号需求集中注册：一趟符号表解析同时满足所有 provider，各查询只做过滤。
        let boringssl_symbols = match context.target_image.arch() {
            Arch::X86_64 => MAP_SYMBOLS_X86_64,
            Arch::Aarch64 => MAP_SYMBOLS_AARCH64,
        };
        let symbol_names = [
            OPENSSL_PROBE_SYMBOLS,
            GNUTLS_SYMBOLS,
            NSS_SYMBOLS,
            boringssl_symbols,
        ]
        .concat();
        context.target_image.register_symbol_names(&symbol_names);
        context.target_image.register_rustls_symbols();

        let registrations: [&dyn ExecutablePatternRegistration; 3] =
            [&self.rustls, &self.openssl, &self.boringssl];
        let detectors: [&dyn ProbeDetector; 6] = [
            &self.rustls,
            &self.openssl,
            &self.boringssl,
            &self.go_tls,
            &self.gnutls,
            &self.nss,
        ];
        let mut outcomes = Vec::new();
        let mut evidence =
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone());
        for detector in detectors {
            // 按 provider 粒度注册：检测前注册本 provider 需要的 patterns，多个
            // pattern 请求共享一趟联合扫描；是否继续跑下一个 provider 由
            // selector 的 should_stop（FirstComplete 匹配即停）在检测后决定，
            // 后验策略（CollectAll 等）则继续收集全部结果。
            if let Some(registration) = registrations
                .iter()
                .find(|registration| registration.detector_path() == detector.path())
            {
                registration.register_executable_patterns(context);
            }
            evidence.facts.push(EvidenceFact {
                key: "provider_detector".to_string(),
                value: detector.path().display(),
            });
            let outcome = detector.detect(context)?;
            let stop = self.selector.should_stop(&outcome);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        Ok(self.selector.select(evidence, outcomes))
    }
}
