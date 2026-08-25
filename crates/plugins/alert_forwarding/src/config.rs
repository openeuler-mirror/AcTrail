use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

pub(super) const ALERT_FORWARDING_CONFIG_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://actrail.local/schemas/alert-forwarding.config.v1.schema.json",
  "title": "AcTrail alert forwarding configuration",
  "type": "object",
  "additionalProperties": false,
  "required": ["enabled", "all_categories", "categories"],
  "properties": {
    "enabled": {
      "type": "boolean",
      "description": "Forward matching alerts while the alert proxy connection is active."
    },
    "all_categories": {
      "type": "boolean",
      "description": "Forward every alert category."
    },
    "categories": {
      "type": "array",
      "description": "Alert category names forwarded when all_categories is false.",
      "maxItems": 256,
      "items": {
        "type": "string",
        "minLength": 1,
        "maxLength": 128,
        "pattern": "^[A-Za-z0-9][A-Za-z0-9._/-]*$"
      },
      "uniqueItems": true
    }
  }
}"#;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlertForwardingConfig {
    enabled: bool,
    all_categories: bool,
    #[serde(default)]
    categories: Vec<String>,
}

impl AlertForwardingConfig {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            all_categories: false,
            categories: Vec::new(),
        }
    }

    pub fn from_json(raw: &str) -> Result<Self, AlertForwardingConfigError> {
        let config = serde_json::from_str::<Self>(raw)
            .map_err(|error| AlertForwardingConfigError::InvalidJson(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_json(&self) -> Result<String, AlertForwardingConfigError> {
        serde_json::to_string_pretty(self)
            .map_err(|error| AlertForwardingConfigError::InvalidJson(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), AlertForwardingConfigError> {
        if self.all_categories && !self.categories.is_empty() {
            return Err(AlertForwardingConfigError::CategoriesWithAllCategories);
        }
        if self.categories.len() > 256 {
            return Err(AlertForwardingConfigError::TooManyCategories(
                self.categories.len(),
            ));
        }
        let mut unique = BTreeSet::new();
        for category in &self.categories {
            if category.len() > 128 || !valid_category(category) {
                return Err(AlertForwardingConfigError::InvalidCategory(
                    category.clone(),
                ));
            }
            if !unique.insert(category.as_str()) {
                return Err(AlertForwardingConfigError::DuplicateCategory(
                    category.clone(),
                ));
            }
        }
        Ok(())
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub const fn all_categories(&self) -> bool {
        self.all_categories
    }

    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    pub(crate) fn with_enabled(&self, enabled: bool) -> Self {
        let mut config = self.clone();
        config.enabled = enabled;
        config
    }
}

impl Default for AlertForwardingConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

fn valid_category(category: &str) -> bool {
    let mut bytes = category.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlertForwardingConfigError {
    InvalidJson(String),
    CategoriesWithAllCategories,
    TooManyCategories(usize),
    InvalidCategory(String),
    DuplicateCategory(String),
}

impl fmt::Display for AlertForwardingConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(formatter, "invalid alert forwarding JSON: {message}")
            }
            Self::CategoriesWithAllCategories => {
                formatter.write_str("categories must be empty when all_categories=true")
            }
            Self::TooManyCategories(count) => write!(
                formatter,
                "alert forwarding category count {count} exceeds maximum 256"
            ),
            Self::InvalidCategory(category) => write!(
                formatter,
                "alert category must match ^[A-Za-z0-9][A-Za-z0-9._/-]{{0,127}}$: {category:?}"
            ),
            Self::DuplicateCategory(category) => {
                write!(formatter, "duplicate alert category: {category}")
            }
        }
    }
}

impl std::error::Error for AlertForwardingConfigError {}
