pub(crate) use config::BoringSslProbeDetectorConfig;
pub(crate) use probe_detector::{
    BoringSslProbeDetector, MAP_SYMBOLS_AARCH64, MAP_SYMBOLS_X86_64, SHARED_SYMBOL_MAP_RESOLVER,
    STATIC_RESOLVER, SYMBOL_MAP_RESOLVER,
};

mod aarch64;
mod common;
mod config;
mod probe_detector;
mod x86_64;
