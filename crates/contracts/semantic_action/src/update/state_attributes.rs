use std::collections::BTreeMap;

use crate::{SemanticActionFinalizationReason, SemanticCommandKind, attr_keys as attrs};

/// Attribute representation of typed command state, shared by delivery and storage.
pub struct SemanticActionStateAttributes<'a> {
    attributes: &'a mut BTreeMap<String, String>,
}

impl<'a> SemanticActionStateAttributes<'a> {
    pub fn new(attributes: &'a mut BTreeMap<String, String>) -> Self {
        Self { attributes }
    }

    pub fn command_kind(&mut self, kind: SemanticCommandKind) {
        self.attributes
            .insert(attrs::invocation::KIND.into(), kind.as_str().into());
    }

    pub fn command_tool_name(&mut self, tool_name: String) {
        self.attributes
            .insert(attrs::command::TOOL_NAME.into(), tool_name);
    }

    pub fn command_exit_code(&mut self, code: i32) {
        self.attributes
            .insert(attrs::command::EXIT_CODE.into(), code.to_string());
    }

    pub fn finalization_reason(&mut self, reason: SemanticActionFinalizationReason) {
        if reason == SemanticActionFinalizationReason::TraceClosed {
            self.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.into(),
                "true".into(),
            );
        }
    }
}
