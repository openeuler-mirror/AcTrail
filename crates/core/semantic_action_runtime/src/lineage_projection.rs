//! Durable process-lineage projection for semantic action links.

mod derive;
mod index;

#[cfg(test)]
mod tests;

use semantic_action::attr_keys as attrs;

pub use derive::derive_lineage_links;

pub const ATTR_LINK_SOURCE: &str = attrs::actrail::LINK_SOURCE;
pub const LINK_SOURCE_PROCESS_LINEAGE: &str = "process_lineage";

const ATTR_AGENT_ACTION_SEQUENCE: &str = attrs::agent::PERFORMED_ACTION_SEQUENCE;
const ATTR_AGENT_IDENTITY_STATUS: &str = attrs::agent::IDENTITY_STATUS;
const ATTR_LINK_VALID: &str = attrs::actrail::LINK_VALID;
const LINK_VALID_FALSE: &str = "false";
