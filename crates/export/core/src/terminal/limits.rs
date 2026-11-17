use crate::ExportError;

/// Bounds delivery identities independently for each export route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalActionLimits {
    pub(super) per_trace: usize,
    pub(super) entries: usize,
    pub(super) key_bytes: usize,
}

impl Default for TerminalActionLimits {
    fn default() -> Self {
        Self {
            per_trace: 65_536,
            entries: 262_144,
            key_bytes: 32 * 1024 * 1024,
        }
    }
}

impl TerminalActionLimits {
    pub fn new(per_trace: usize, entries: usize, key_bytes: usize) -> Result<Self, ExportError> {
        if per_trace == 0 || entries == 0 || key_bytes == 0 || per_trace > entries {
            return Err(ExportError::new(
                "terminal_action_limits",
                "terminal action limits must be positive and per-trace entries must not exceed total entries",
            ));
        }
        Ok(Self {
            per_trace,
            entries,
            key_bytes,
        })
    }

    pub fn max_entries_per_trace(self) -> usize {
        self.per_trace
    }
    pub fn max_entries(self) -> usize {
        self.entries
    }
    pub fn max_key_bytes(self) -> usize {
        self.key_bytes
    }
}
