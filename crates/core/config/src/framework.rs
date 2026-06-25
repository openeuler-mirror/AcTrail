//! Typed configuration loading primitives.

use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    source: Option<String>,
    message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            source: None,
            message: message.into(),
        }
    }

    pub fn with_source(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            source: Some(source.into()),
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.source {
            Some(source) => write!(formatter, "{source}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

pub trait ConfigModel: Sized + Default + DeserializeOwned + Serialize {
    const MODEL_NAME: &'static str;

    fn validate(&self) -> Result<(), ConfigError> {
        Ok(())
    }

    fn from_toml(raw: &str) -> Result<Self, ConfigError> {
        let model = toml::from_str::<Self>(raw)
            .map_err(|error| ConfigError::with_source(Self::MODEL_NAME, error.to_string()))?;
        model.validate()?;
        Ok(model)
    }

    fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        toml::to_string_pretty(self)
            .map_err(|error| ConfigError::with_source(Self::MODEL_NAME, error.to_string()))
    }
}

pub trait ConfigSource {
    fn label(&self) -> String;
    fn read_to_string(&self) -> Result<String, ConfigError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileConfigSource {
    path: PathBuf,
}

impl FileConfigSource {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl ConfigSource for FileConfigSource {
    fn label(&self) -> String {
        self.path.display().to_string()
    }

    fn read_to_string(&self) -> Result<String, ConfigError> {
        fs::read_to_string(&self.path)
            .map_err(|error| ConfigError::with_source(self.label(), error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineConfigSource {
    label: String,
    raw: String,
}

impl InlineConfigSource {
    pub fn new(label: impl Into<String>, raw: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            raw: raw.into(),
        }
    }
}

impl ConfigSource for InlineConfigSource {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn read_to_string(&self) -> Result<String, ConfigError> {
        Ok(self.raw.clone())
    }
}

pub struct ConfigLoader<T> {
    sources: Vec<Box<dyn ConfigSource>>,
    _model: PhantomData<T>,
}

impl<T> Default for ConfigLoader<T> {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            _model: PhantomData,
        }
    }
}

impl<T> ConfigLoader<T>
where
    T: ConfigModel,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_source(mut self, source: impl ConfigSource + 'static) -> Self {
        self.sources.push(Box::new(source));
        self
    }

    pub fn load(self) -> Result<T, ConfigError> {
        let Some(source) = self.sources.into_iter().last() else {
            let model = T::default();
            model.validate()?;
            return Ok(model);
        };
        let raw = source.read_to_string()?;
        T::from_toml(&raw).map_err(|error| ConfigError::with_source(source.label(), error.message))
    }
}
