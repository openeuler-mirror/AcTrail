use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use config_core::daemon::OperatorConfig;
use resource_profiling::ResourceProfilingConfig;
use serde_json::json;
use toml_edit::{DocumentMut, Item, Value};

#[path = "configuration/fields.rs"]
mod fields;
#[path = "configuration/file.rs"]
mod file;

use fields::EDITABLE_FIELDS;

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
    save_lock: Mutex<()>,
}

impl ConfigEditor {
    pub(super) fn file(path: PathBuf) -> Self {
        Self {
            store: Box::new(file::FileConfigStore::new(path)),
            save_lock: Mutex::new(()),
        }
    }

    pub(super) fn read(&self) -> Result<String, ConfigSaveError> {
        self.read_document()?
            .json(self.store.path())
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
        let edits = ConfigEdits::parse(body).map_err(ConfigSaveError::Invalid)?;
        let _guard = self
            .save_lock
            .lock()
            .map_err(|_| ConfigSaveError::Storage("config save lock unavailable".into()))?;
        let mut document = self.read_document()?;
        document.apply(&edits).map_err(ConfigSaveError::Invalid)?;
        let text = document.document.to_string();
        let validated = ConfigDocument::parse(&text).map_err(ConfigSaveError::Invalid)?;
        let response = validated
            .json(self.store.path())
            .map_err(ConfigSaveError::Storage)?;
        self.store
            .replace(&text)
            .map_err(ConfigSaveError::Storage)?;
        Ok(response)
    }
}

struct ConfigEdits(BTreeMap<String, Value>);

impl ConfigEdits {
    fn parse(body: &[u8]) -> Result<Self, String> {
        let fields: BTreeMap<String, serde_json::Value> = serde_json::from_slice(body)
            .map_err(|error| format!("invalid config edits JSON: {error}"))?;
        if fields.is_empty() {
            return Err("no editable config fields supplied".into());
        }
        let mut edits = BTreeMap::new();
        for (field, value) in fields {
            let spec = EDITABLE_FIELDS
                .iter()
                .find(|spec| spec.key == field)
                .ok_or_else(|| format!("config field {field} is read-only or unknown"))?;
            edits.insert(field, spec.decode(&value)?);
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

    fn json(&self, path: &Path) -> Result<String, String> {
        let profiling = ResourceProfilingConfig::parse(&self.config.resource_profiling)?;
        let normalized: DocumentMut = self
            .config
            .to_hierarchical_toml()?
            .parse()
            .map_err(|error| format!("render config TOML failed: {error}"))?;
        // Runtime configuration can omit an inactive backend's settings. Keep
        // their defaults sourced from the same domain template used by init.
        let defaults = if EDITABLE_FIELDS.iter().any(|field| {
            Self::field(&self.document, field.key).is_none()
                && Self::field(&normalized, field.key).is_none()
        }) {
            Some(
                OperatorConfig::default_hierarchical_template()?
                    .parse::<DocumentMut>()
                    .map_err(|error| format!("render config defaults failed: {error}"))?,
            )
        } else {
            None
        };
        let mut fields = BTreeMap::new();
        for spec in &EDITABLE_FIELDS {
            let field = spec.key;
            if let Some(value) = spec.encode_resource(&profiling) {
                fields.insert(field, value);
                continue;
            }
            let item = Self::field(&self.document, field)
                .or_else(|| Self::field(&normalized, field))
                .or_else(|| {
                    defaults
                        .as_ref()
                        .and_then(|defaults| Self::field(defaults, field))
                })
                .ok_or_else(|| format!("missing editable config field {field}"))?;
            fields.insert(field, spec.encode(item)?);
        }
        Ok(json!({
            "available": true,
            "source": { "mode": "operator_config", "path": path.display().to_string() },
            "fields": fields,
        })
        .to_string())
    }

    fn field<'a>(document: &'a DocumentMut, field: &str) -> Option<&'a Item> {
        let mut item = document.as_item();
        for part in field.split('.') {
            item = item.get(part)?;
        }
        Some(item)
    }
}
