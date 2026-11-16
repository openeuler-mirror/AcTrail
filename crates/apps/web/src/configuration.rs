use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use config_core::daemon::OperatorConfig;
use serde_json::json;
use toml_edit::{DocumentMut, Item, Value};

#[path = "configuration/fields.rs"]
mod fields;
#[path = "configuration/file.rs"]
mod file;

use fields::ConfigFields;

trait ConfigStore: Send + Sync {
    fn path(&self) -> &Path;
    fn read(&self) -> Result<String, String>;
    fn replace(&self, text: &str) -> Result<(), String>;
}

pub(super) enum ConfigSaveError {
    Invalid(String),
    ExistingInvalid(String),
    Storage(String),
}

pub(super) struct ConfigEditor {
    store: Box<dyn ConfigStore>,
    fields: ConfigFields,
    save_lock: Mutex<()>,
}

impl ConfigEditor {
    pub(super) fn file(path: PathBuf) -> Result<Self, String> {
        Ok(Self {
            store: Box::new(file::FileConfigStore::new(path)),
            fields: ConfigFields::load()?,
            save_lock: Mutex::new(()),
        })
    }

    pub(super) fn read(&self) -> Result<String, ConfigSaveError> {
        self.read_document()?
            .json(self.store.path(), &self.fields)
            .map_err(ConfigSaveError::Storage)
    }

    fn read_document(&self) -> Result<ConfigDocument, ConfigSaveError> {
        let text = self.store.read().map_err(ConfigSaveError::Storage)?;
        ConfigDocument::parse(&text).map_err(|error| {
            ConfigSaveError::ExistingInvalid(format!(
                "invalid existing configuration {}: {error}",
                self.store.path().display()
            ))
        })
    }

    pub(super) fn save(&self, body: &[u8]) -> Result<String, ConfigSaveError> {
        let edits = ConfigEdits::parse(body, &self.fields).map_err(ConfigSaveError::Invalid)?;
        let _guard = self
            .save_lock
            .lock()
            .map_err(|_| ConfigSaveError::Storage("config save lock unavailable".into()))?;
        let mut document = self.read_document()?;
        document.apply(&edits).map_err(ConfigSaveError::Invalid)?;
        let text = document.document.to_string();
        let validated = ConfigDocument::parse(&text).map_err(ConfigSaveError::Invalid)?;
        let response = validated
            .json(self.store.path(), &self.fields)
            .map_err(ConfigSaveError::Storage)?;
        self.store
            .replace(&text)
            .map_err(ConfigSaveError::Storage)?;
        Ok(response)
    }
}

struct ConfigEdits(BTreeMap<String, Value>);

impl ConfigEdits {
    fn parse(body: &[u8], catalog: &ConfigFields) -> Result<Self, String> {
        let fields: BTreeMap<String, serde_json::Value> = serde_json::from_slice(body)
            .map_err(|error| format!("invalid config edits JSON: {error}"))?;
        if fields.is_empty() {
            return Err("no editable config fields supplied".into());
        }
        let mut edits = BTreeMap::new();
        for (field, value) in fields {
            let spec = catalog.spec(&field)?;
            let value = spec.decode(&value)?;
            edits.insert(field, value);
        }
        Ok(Self(edits))
    }
}

struct ConfigDocument {
    document: DocumentMut,
    config: OperatorConfig,
}

impl ConfigDocument {
    fn parse(text: &str) -> Result<Self, String> {
        let config = OperatorConfig::parse(text)?;
        let document = text
            .parse()
            .map_err(|error| format!("invalid config TOML: {error}"))?;
        Ok(Self { document, config })
    }

    fn apply(&mut self, edits: &ConfigEdits) -> Result<(), String> {
        for (field, replacement) in &edits.0 {
            let mut replacement = replacement.clone();
            let mut item = self.document.as_item_mut();
            for part in field.split('.') {
                if !item.is_none() && !item.is_table() && item.as_inline_table().is_none() {
                    return Err(format!(
                        "cannot edit {field}: its parent is not a TOML table"
                    ));
                }
                item = &mut item[part];
            }
            if let Some(value) = item.as_value() {
                *replacement.decor_mut() = value.decor().clone();
            }
            *item = Item::Value(replacement);
        }
        Ok(())
    }

    fn json(&self, path: &Path, catalog: &ConfigFields) -> Result<String, String> {
        let normalized: DocumentMut = self
            .config
            .to_hierarchical_toml()?
            .parse()
            .map_err(|error| format!("render config TOML failed: {error}"))?;
        let mut fields = BTreeMap::new();
        let mut kinds = BTreeMap::new();
        for spec in catalog.specs() {
            let spec = spec?;
            let field = spec.key;
            kinds.insert(field, spec.kind());
            // Owners may leave inactive fields unvalidated. Prefer encodable source
            // values, then the owner's effective or default representation.
            let encode = |item: Option<&Item>| item.and_then(|item| spec.encode(item));
            let value = encode(ConfigFields::item(&self.document, field))
                .or_else(|| encode(ConfigFields::item(&normalized, field)))
                .or_else(|| encode(catalog.default(field)))
                .ok_or_else(|| format!("missing editable config field {field}"))?;
            fields.insert(field, value);
        }
        Ok(json!({
            "available": true,
            "source": { "mode": "operator_config", "path": path.display().to_string() },
            "fields": fields,
            "kinds": kinds,
        })
        .to_string())
    }
}
