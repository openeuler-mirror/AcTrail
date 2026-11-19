pub(crate) use config::X86_64BoringSslProbeDetectorConfig;
pub(crate) use probe_detector::X86_64BoringSslProbeDetector;

mod build_verified_read_write_pair;
mod config;
mod probe_detector;
mod shared_library;
mod static_pattern;
mod symbol;
