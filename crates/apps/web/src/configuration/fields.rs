use config_core::daemon::OperatorConfig;
use config_export_filter::ConfigExportFilter;
use serde_json::{Value as JsonValue, json};
use toml_edit::{Array, DocumentMut, Item, Value};

use FieldType::{Boolean, Integer, StringArray, Text, Toml};

pub(super) struct ConfigFields {
    filter: ConfigExportFilter,
    defaults: DocumentMut,
}

impl ConfigFields {
    pub(super) fn load() -> Result<Self, String> {
        let template = OperatorConfig::default_hierarchical_template()?;
        let value: toml::Value = template
            .parse()
            .map_err(|error| format!("parse config defaults failed: {error}"))?;
        let filter = ConfigExportFilter::bundled(&value)?;
        let defaults = toml_edit::ser::to_document(&filter.project(&value)?)
            .map_err(|error| format!("construct editable config defaults failed: {error}"))?;
        let fields = Self { filter, defaults };
        for spec in fields.specs() {
            spec?;
        }
        Ok(fields)
    }

    pub(super) fn specs(&self) -> impl Iterator<Item = Result<ConfigField<'_>, String>> {
        self.filter.paths().map(|key| self.spec(key))
    }

    pub(super) fn spec<'a>(&'a self, key: &'a str) -> Result<ConfigField<'a>, String> {
        if !self.filter.allows(key) {
            return Err(format!("config field {key} is read-only or unknown"));
        }
        let item = self
            .default(key)
            .ok_or_else(|| format!("editable config field {key} has no owner default value"))?;
        ConfigField::from_default(key, item)
    }

    pub(super) fn default(&self, key: &str) -> Option<&Item> {
        Self::item(&self.defaults, key)
    }

    pub(super) fn item<'a>(document: &'a DocumentMut, key: &str) -> Option<&'a Item> {
        key.split('.')
            .try_fold(document.as_item(), |item, part| item.get(part))
    }
}

enum FieldType {
    Text,
    Integer,
    Boolean,
    StringArray,
    Toml,
}

pub(super) struct ConfigField<'a> {
    pub(super) key: &'a str,
    kind: FieldType,
}

impl<'a> ConfigField<'a> {
    fn from_default(key: &'a str, item: &Item) -> Result<Self, String> {
        let kind = if item.as_str().is_some() {
            Text
        } else if item.as_integer().is_some() {
            Integer
        } else if item.as_bool().is_some() {
            Boolean
        } else if item.as_array().is_some_and(|array| {
            (OperatorConfig::is_string_array_field(key) || !array.is_empty())
                && array.iter().all(|value| value.as_str().is_some())
        }) {
            StringArray
        } else if item.is_value() || item.is_array_of_tables() {
            Toml
        } else {
            // TODO: define export/edit semantics for absent optional values.
            return Err(format!(
                "editable config field {key} requires a text, integer, boolean or typed string-array owner default"
            ));
        };
        Ok(Self { key, kind })
    }

    pub(super) fn kind(&self) -> &'static str {
        match self.kind {
            Text => "text",
            Integer => "integer",
            Boolean => "boolean",
            StringArray => "string_array",
            Toml => "toml",
        }
    }

    pub(super) fn decode(&self, value: &JsonValue) -> Result<Value, String> {
        let invalid = |expected| format!("invalid {}: expected {expected}", self.key);
        match self.kind {
            Text => value
                .as_str()
                .map(Value::from)
                .ok_or_else(|| invalid("a string")),
            Boolean => value
                .as_bool()
                .map(Value::from)
                .ok_or_else(|| invalid("a boolean")),
            Integer => {
                // TOML integers are signed. The owning configuration module validates
                // signedness and range when ConfigDocument::parse checks the edits;
                // valid settings include -1 depths and negative compression levels.
                let integer = value.as_i64().ok_or_else(|| invalid("a TOML integer"))?;
                // The inactive SQLite backend is omitted by domain parsing.
                if self.key == "storage.sqlite.busy_timeout_ms" && integer <= 0 {
                    return Err(invalid("a positive TOML integer"));
                }
                Ok(Value::from(integer))
            }
            StringArray => {
                let values = value
                    .as_array()
                    .ok_or_else(|| invalid("an array of strings"))?;
                let mut array = Array::new();
                for value in values {
                    array.push(
                        value
                            .as_str()
                            .ok_or_else(|| invalid("an array of strings"))?,
                    );
                }
                Ok(Value::Array(array))
            }
            Toml => value
                .as_str()
                .ok_or_else(|| invalid("a TOML value string"))?
                .trim()
                .parse::<Value>()
                .map_err(|error| format!("invalid {} TOML value: {error}", self.key)),
        }
    }

    pub(super) fn encode(&self, item: &Item) -> Option<JsonValue> {
        match self.kind {
            Text => item.as_str().map(|value| json!(value)),
            // Decimal text keeps every TOML i64 exact in the browser. POST
            // still requires a JSON integer and preserves the file's type.
            Integer => item.as_integer().map(|value| json!(value.to_string())),
            Boolean => item.as_bool().map(|value| json!(value)),
            StringArray => item.as_array().and_then(|array| {
                array
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()
                    .map(|values| json!(values))
            }),
            Toml => item
                .clone()
                .into_value()
                .ok()
                .map(|value| json!(value.to_string().trim())),
        }
    }
}
