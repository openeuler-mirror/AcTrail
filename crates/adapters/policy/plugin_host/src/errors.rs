//! Error boundaries for plugin loading and evaluation.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginHostError {
    pub stage: String,
    pub message: String,
}

impl PluginHostError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}
