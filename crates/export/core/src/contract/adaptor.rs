use crate::{ExportError, SemanticActionExportRecord};

pub trait SemanticActionExportAdapter {
    type Message: Send + 'static;

    fn name(&self) -> &'static str;

    fn encode(&self, record: SemanticActionExportRecord<'_>) -> Result<Self::Message, ExportError>;
}
