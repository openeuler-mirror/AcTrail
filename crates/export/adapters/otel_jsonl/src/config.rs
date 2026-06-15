use std::collections::BTreeMap;
use std::path::PathBuf;

pub const OTEL_JSONL_ROUTE_KIND: &str = "otel-jsonl";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelJsonlExporterConfig {
    pub path: PathBuf,
    pub overwrite_enabled: bool,
    pub queue_capacity: u32,
    pub flush_every_spans: u32,
}

impl OtelJsonlExporterConfig {
    pub fn parse_section(
        section_name: &str,
        entries: Vec<(String, String)>,
    ) -> Result<Self, String> {
        let values = OtelJsonlConfigValues::from_entries(section_name, entries)?;
        let config = Self {
            path: values.required_path_buf("path")?,
            overwrite_enabled: values.required_bool("overwrite_enabled")?,
            queue_capacity: values.required_positive_u32("queue_capacity")?,
            flush_every_spans: values.required_positive_u32("flush_every_spans")?,
        };
        Ok(config)
    }

    pub fn validate_enabled_route(&self) -> Result<(), String> {
        if !self.path.is_absolute() {
            return Err("invalid otel-jsonl export path: expected absolute path".to_string());
        }
        Ok(())
    }
}

struct OtelJsonlConfigValues {
    section_name: String,
    values: BTreeMap<String, String>,
}

impl OtelJsonlConfigValues {
    fn from_entries(section_name: &str, entries: Vec<(String, String)>) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for (key, value) in entries {
            reject_unknown_key(section_name, &key)?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate config key {section_name}.{key}"));
            }
        }
        Ok(Self {
            section_name: section_name.to_string(),
            values,
        })
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {}.{key}", self.section_name))
    }

    fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {}.{key}: expected true or false, got {value}",
                self.section_name
            )),
        }
    }

    fn required_path_buf(&self, key: &'static str) -> Result<PathBuf, String> {
        self.required(key).map(PathBuf::from)
    }

    fn required_positive_u32(&self, key: &'static str) -> Result<u32, String> {
        let value = self
            .required(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {}.{key}: {error}", self.section_name))?;
        if value == u32::default() {
            return Err(format!(
                "invalid {}.{key}: value must be positive",
                self.section_name
            ));
        }
        Ok(value)
    }
}

fn reject_unknown_key(section_name: &str, key: &str) -> Result<(), String> {
    match key {
        "path" | "overwrite_enabled" | "queue_capacity" | "flush_every_spans" => Ok(()),
        _ => Err(format!("unknown config key {section_name}.{key}")),
    }
}

#[cfg(test)]
mod tests {
    use super::OtelJsonlExporterConfig;

    #[test]
    fn unknown_otel_jsonl_key_is_rejected() {
        let error = OtelJsonlExporterConfig::parse_section(
            "export.routes.otel-jsonl.live-otel",
            vec![
                (
                    "path".to_string(),
                    "/tmp/actrail-live-spans.otlp.jsonl".to_string(),
                ),
                ("overwrite_enabled".to_string(), "true".to_string()),
                ("queue_capacity".to_string(), "1024".to_string()),
                ("flush_every_spans".to_string(), "1".to_string()),
                ("unexpected".to_string(), "true".to_string()),
            ],
        )
        .expect_err("unknown OTEL JSONL config key should fail");

        assert!(error.contains("unknown config key export.routes.otel-jsonl.live-otel.unexpected"));
    }
}
