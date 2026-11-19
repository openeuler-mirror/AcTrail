use semantic_action::{
    SemanticAction, SemanticActionChange, SemanticActionLink, SemanticActionStateAttributes,
    SemanticActionUpdate, SemanticEvidence, attr_keys as attrs,
};

use super::sizing::AccountedBytes;

// Opening facts are retained only for delivery. Lifecycle/classification remain
// producer-owned; a completed record moves out rather than becoming live state.
pub(super) struct PendingCommand {
    pub(super) action: SemanticAction,
    pub(super) incoming_links: Vec<SemanticActionLink>,
    pub(super) bytes: usize,
}

impl PendingCommand {
    pub(super) fn new(action: &SemanticAction, bytes: usize) -> Self {
        Self {
            action: action.clone(),
            incoming_links: Vec::new(),
            bytes,
        }
    }

    pub(super) fn link_bytes_after(&self, link: &SemanticActionLink) -> usize {
        let previous = self.incoming_links.iter().find(|current| {
            current.parent_action_id == link.parent_action_id && current.role == link.role
        });
        self.bytes
            .saturating_sub(previous.map(AccountedBytes::link).unwrap_or(0))
            .saturating_add(AccountedBytes::link(link))
    }

    pub(super) fn observe_link(&mut self, link: &SemanticActionLink, bytes: usize) {
        if let Some(current) = self.incoming_links.iter_mut().find(|current| {
            current.parent_action_id == link.parent_action_id && current.role == link.role
        }) {
            current.clone_from(link);
        } else {
            self.incoming_links.push(link.clone());
        }
        self.bytes = bytes;
    }

    pub(super) fn metadata_bytes_after(&self, update: &SemanticActionUpdate) -> Option<usize> {
        let (key, value) = match &update.change {
            SemanticActionChange::CommandClassification { kind } => {
                (attrs::invocation::KIND, kind.as_str())
            }
            SemanticActionChange::CommandToolName { tool_name } => {
                (attrs::command::TOOL_NAME, tool_name.as_str())
            }
            _ => return None,
        };
        let previous = self
            .action
            .attributes
            .get(key)
            .map(|value| AccountedBytes::attribute(key, value))
            .unwrap_or(0);
        let mut bytes = self
            .bytes
            .saturating_sub(previous)
            .saturating_add(AccountedBytes::attribute(key, value));
        for (index, item) in update.evidence.iter().enumerate() {
            if !self.action.evidence.contains(item) && !update.evidence[..index].contains(item) {
                bytes = bytes.saturating_add(AccountedBytes::evidence_item(item));
            }
        }
        Some(bytes)
    }

    pub(super) fn observe_metadata(&mut self, update: &SemanticActionUpdate, bytes: usize) {
        let mut attributes = SemanticActionStateAttributes::new(&mut self.action.attributes);
        match &update.change {
            SemanticActionChange::CommandClassification { kind } => attributes.command_kind(*kind),
            SemanticActionChange::CommandToolName { tool_name } => {
                attributes.command_tool_name(tool_name.clone())
            }
            _ => return,
        }
        self.append_evidence(&update.evidence);
        self.bytes = bytes;
    }

    pub(super) fn finish(&mut self, update: &SemanticActionUpdate) {
        let SemanticActionChange::Lifecycle {
            end_time,
            status,
            completeness,
            finalization_reason,
            command_exit_code,
        } = update.change
        else {
            return;
        };
        self.action.end_time = end_time;
        self.action.status = status;
        self.action.completeness = completeness;
        let mut attributes = SemanticActionStateAttributes::new(&mut self.action.attributes);
        if let Some(code) = command_exit_code {
            attributes.command_exit_code(code);
        }
        if let Some(reason) = finalization_reason {
            attributes.finalization_reason(reason);
        }
        self.append_evidence(&update.evidence);
    }

    fn append_evidence(&mut self, evidence: &[SemanticEvidence]) {
        for item in evidence {
            if !self.action.evidence.contains(item) {
                self.action.evidence.push(item.clone());
            }
        }
    }
}
