//! Durable process-lineage projection for semantic action links.

mod derive;
mod index;

#[cfg(test)]
mod tests;

pub use derive::derive_lineage_links;

pub const ATTR_LINK_SOURCE: &str = "actrail.link.source";
pub const LINK_SOURCE_PROCESS_LINEAGE: &str = "process_lineage";

const ATTR_AGENT_ACTION_SEQUENCE: &str = "agent.performed_action.sequence";
const ATTR_AGENT_IDENTITY_STATUS: &str = "agent.identity.status";
const ATTR_LINK_VALID: &str = "actrail.link.valid";
const ATTR_PROCESS_PARENT_IDENTITY_STATE: &str = "process.parent.identity_state";
const LINK_VALID_FALSE: &str = "false";
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";
