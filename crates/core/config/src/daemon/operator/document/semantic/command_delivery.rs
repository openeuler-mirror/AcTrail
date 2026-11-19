use super::*;
use crate::daemon::CommandDeliveryConfig;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct CommandDeliveryDocument {
    max_pending_commands_per_trace: u32,
    max_pending_commands: u32,
    max_accounted_bytes: u64,
}

impl Default for CommandDeliveryDocument {
    fn default() -> Self {
        Self::from_config(CommandDeliveryConfig::default())
    }
}

impl CommandDeliveryDocument {
    pub(super) fn from_config(config: CommandDeliveryConfig) -> Self {
        Self {
            max_pending_commands_per_trace: config.max_pending_commands_per_trace,
            max_pending_commands: config.max_pending_commands,
            max_accounted_bytes: config.max_accounted_bytes,
        }
    }

    pub(super) fn to_config(&self) -> Result<CommandDeliveryConfig, String> {
        let per_trace = require_positive_u32(
            "semantic_retention.command_delivery.max_pending_commands_per_trace",
            self.max_pending_commands_per_trace,
        )?;
        let total = require_positive_u32(
            "semantic_retention.command_delivery.max_pending_commands",
            self.max_pending_commands,
        )?;
        if per_trace > total {
            return Err("semantic_retention.command_delivery.max_pending_commands_per_trace must not exceed max_pending_commands".into());
        }
        let bytes = require_positive_u64(
            "semantic_retention.command_delivery.max_accounted_bytes",
            self.max_accounted_bytes,
        )?;
        usize::try_from(bytes).map_err(|_| {
            "semantic_retention.command_delivery.max_accounted_bytes exceeds addressable memory"
                .to_string()
        })?;
        Ok(CommandDeliveryConfig {
            max_pending_commands_per_trace: per_trace,
            max_pending_commands: total,
            max_accounted_bytes: bytes,
        })
    }
}
