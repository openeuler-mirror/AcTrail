//! Durable process-lineage projection for semantic action links.

mod derive;
mod index;

use semantic_action::attr_keys as attrs;

pub use derive::derive_lineage_links;

pub const ATTR_LINK_SOURCE: &str = attrs::actrail::LINK_SOURCE;
pub const LINK_SOURCE_PROCESS_LINEAGE: &str = "process_lineage";

const ATTR_AGENT_ACTION_SEQUENCE: &str = attrs::agent::PERFORMED_ACTION_SEQUENCE;
const ATTR_PROCESS_PARENT_IDENTITY_STATE: &str = attrs::process_parent::IDENTITY_STATE;
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";
