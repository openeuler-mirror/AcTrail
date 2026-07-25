pub(crate) use config::GoTlsProbeDetectorConfig;
pub(crate) use probe_detector::{
    GoTlsProbeDetector, READ_SYMBOL, RESOLVER, RUNTIME_MEMMOVE_SYMBOL, SYMBOLS, WRITE_SYMBOL,
};

mod config;
mod pclntab;
mod probe_detector;
