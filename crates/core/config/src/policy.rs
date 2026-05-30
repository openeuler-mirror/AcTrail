//! Policy configuration shared across ingest and payload handling.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvCaptureMode {
    Disabled,
    KeysOnly,
    AllowlistedValues,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathHandlingMode {
    Preserve,
    RedactSensitiveSegments,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ByteLimit {
    pub bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPolicy {
    pub capture_limit: ByteLimit,
    pub preserve_original_size: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyConfig {
    pub env_capture_mode: EnvCaptureMode,
    pub path_handling_mode: PathHandlingMode,
    pub stdout: StreamPolicy,
    pub stderr: StreamPolicy,
    pub allow_payload_capture: bool,
}
