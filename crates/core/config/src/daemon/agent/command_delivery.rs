#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandDeliveryConfig {
    pub max_pending_commands_per_trace: u32,
    pub max_pending_commands: u32,
    pub max_accounted_bytes: u64,
}

impl Default for CommandDeliveryConfig {
    fn default() -> Self {
        Self {
            max_pending_commands_per_trace: 4_096,
            max_pending_commands: 16_384,
            max_accounted_bytes: 64 * 1024 * 1024,
        }
    }
}
