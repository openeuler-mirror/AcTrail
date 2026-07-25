use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::ElfImage;

pub(crate) struct BoringSslSymbolEvidence {
    required: &'static [&'static str],
}

impl BoringSslSymbolEvidence {
    pub(crate) fn new(required: &'static [&'static str]) -> Self {
        Self { required }
    }

    pub(crate) fn resolve(&self, image: &ElfImage) -> ToolResult<Option<BTreeMap<String, u64>>> {
        let symbols = image.unique_defined_symbol_values(self.required)?;
        Ok(self
            .required
            .iter()
            .all(|symbol| symbols.contains_key(*symbol))
            .then_some(symbols))
    }
}
