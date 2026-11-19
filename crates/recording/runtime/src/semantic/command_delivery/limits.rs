use crate::RecordingError;

#[derive(Clone, Copy, Debug)]
pub struct CommandDeliveryLimits {
    pub(super) per_trace: usize,
    pub(super) total: usize,
    pub(super) bytes: usize,
}

impl CommandDeliveryLimits {
    pub fn new(per_trace: usize, total: usize, bytes: usize) -> Result<Self, RecordingError> {
        if per_trace == 0 || total == 0 || bytes == 0 || per_trace > total {
            return Err(RecordingError::new(
                "command_delivery",
                "command delivery limits must be positive and per-trace entries must not exceed total entries",
            ));
        }
        Ok(Self {
            per_trace,
            total,
            bytes,
        })
    }
}
