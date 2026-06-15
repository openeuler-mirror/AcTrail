use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use export_otel_jsonl::{OTEL_JSONL_ROUTE_KIND, OtelJsonlExporterConfig};

use crate::{
    ExportConfig, ExportDeliveryConfig, ExportRouteConfig, ExportRouteKind, ExportRouteTargetConfig,
};

const EXPORT_SECTION: &str = "export";
const EXPORT_ROUTE_ADAPTER_PREFIX: &str = "export.routes.";
const LEGACY_OTEL_PREFIX: &str = "otel_live_export_";

pub(crate) fn parse_export_config(raw: &str) -> Result<ExportConfig, String> {
    let values = ExportConfigValues::parse(raw)?;
    let enabled = values.export.required_bool("enabled")?;
    let mut seen_names = BTreeSet::new();
    let mut routes = Vec::new();
    for route in values.routes {
        let route = parse_route(route, &values.adapter_sections)?;
        if !seen_names.insert(route.name.clone()) {
            return Err(format!("duplicate export route name {}", route.name));
        }
        routes.push(route);
    }
    if enabled && routes.iter().all(|route| !route.enabled) {
        return Err("export.enabled=true requires at least one enabled export route".to_string());
    }
    if enabled {
        for route in routes.iter().filter(|route| route.enabled) {
            route.target.validate_enabled_route()?;
        }
    }
    Ok(ExportConfig::new(enabled, routes))
}

fn parse_route(
    values: ExportRouteValues,
    adapter_sections: &BTreeMap<AdapterSectionKey, ConfigSection>,
) -> Result<ExportRouteConfig, String> {
    let name = values.required("name")?;
    let kind = values.required_parsed::<ExportRouteKind>("kind")?;
    let delivery = values.required_parsed::<ExportDeliveryConfig>("delivery")?;
    let enabled = values.required_bool("enabled")?;
    let adapter_key = AdapterSectionKey {
        kind: kind.as_str().to_string(),
        route_name: name.clone(),
    };
    let adapter = adapter_sections
        .get(&adapter_key)
        .ok_or_else(|| format!("missing [export.routes.{}.{}]", kind.as_str(), name))?;
    let target = match kind {
        ExportRouteKind::OtelJsonl => ExportRouteTargetConfig::OtelJsonl(
            OtelJsonlExporterConfig::parse_section(&adapter.qualified_name(), adapter.entries())?,
        ),
    };
    Ok(ExportRouteConfig {
        name,
        enabled,
        delivery,
        target,
    })
}

struct ExportConfigValues {
    export: ConfigSection,
    routes: Vec<ExportRouteValues>,
    adapter_sections: BTreeMap<AdapterSectionKey, ConfigSection>,
}

impl ExportConfigValues {
    fn parse(raw: &str) -> Result<Self, String> {
        let mut current = Section::Root;
        let mut export = None;
        let mut routes = Vec::<ExportRouteValues>::new();
        let mut adapter_sections = BTreeMap::<AdapterSectionKey, ConfigSection>::new();
        for (line_index, line) in raw.lines().enumerate() {
            let line_number = line_index + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(section) = parse_section_header(trimmed, line_number)? {
                current = section;
                match &current {
                    Section::Export => {
                        export.get_or_insert_with(|| ConfigSection::new(EXPORT_SECTION));
                    }
                    Section::Route => routes.push(ExportRouteValues::new(routes.len())),
                    Section::Adapter { kind, route_name } => {
                        let key = AdapterSectionKey {
                            kind: kind.clone(),
                            route_name: route_name.clone(),
                        };
                        let section_name = format!("export.routes.{}.{}", kind, route_name);
                        if adapter_sections
                            .insert(key, ConfigSection::new(section_name))
                            .is_some()
                        {
                            return Err(format!("duplicate section {trimmed}"));
                        }
                    }
                    Section::Root | Section::Other => {}
                }
                continue;
            }
            reject_legacy_key(trimmed)?;
            let Some((key, value)) = parse_key_value(trimmed, line_number)? else {
                continue;
            };
            match &current {
                Section::Export => export
                    .get_or_insert_with(|| ConfigSection::new(EXPORT_SECTION))
                    .insert(key, value)?,
                Section::Route => routes
                    .last_mut()
                    .ok_or_else(|| "internal export route parser state error".to_string())?
                    .insert(key, value)?,
                Section::Adapter { kind, route_name } => {
                    let key_id = AdapterSectionKey {
                        kind: kind.clone(),
                        route_name: route_name.clone(),
                    };
                    adapter_sections
                        .get_mut(&key_id)
                        .ok_or_else(|| "internal export adapter parser state error".to_string())?
                        .insert(key, value)?;
                }
                Section::Root | Section::Other => {}
            }
        }
        let export = export.ok_or_else(|| "missing [export] section".to_string())?;
        reject_unused_adapter_sections(&routes, &adapter_sections)?;
        Ok(Self {
            export,
            routes,
            adapter_sections,
        })
    }
}

fn reject_unused_adapter_sections(
    routes: &[ExportRouteValues],
    adapter_sections: &BTreeMap<AdapterSectionKey, ConfigSection>,
) -> Result<(), String> {
    let mut route_names = BTreeSet::new();
    for route in routes {
        if let Some(name) = route.values.get("name") {
            route_names.insert(name.clone());
        }
    }
    for key in adapter_sections.keys() {
        if key.kind != OTEL_JSONL_ROUTE_KIND || !route_names.contains(&key.route_name) {
            return Err(format!(
                "adapter section [export.routes.{}.{}] has no matching export route",
                key.kind, key.route_name
            ));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ConfigSection {
    name: String,
    values: BTreeMap<String, String>,
}

impl ConfigSection {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            values: BTreeMap::new(),
        }
    }

    fn insert(&mut self, key: String, value: String) -> Result<(), String> {
        reject_export_key(&self.name, &key)?;
        if self.values.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate config key {}.{}", self.name, key));
        }
        Ok(())
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {}.{}", self.name, key))
    }

    fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {}.{}: expected true or false, got {value}",
                self.name, key
            )),
        }
    }

    fn entries(&self) -> Vec<(String, String)> {
        self.values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn qualified_name(&self) -> String {
        self.name.clone()
    }
}

struct ExportRouteValues {
    index: usize,
    values: BTreeMap<String, String>,
}

impl ExportRouteValues {
    fn new(index: usize) -> Self {
        Self {
            index,
            values: BTreeMap::new(),
        }
    }

    fn insert(&mut self, key: String, value: String) -> Result<(), String> {
        reject_route_key(&key)?;
        if self.values.insert(key.clone(), value).is_some() {
            return Err(format!(
                "duplicate config key export.routes[{}].{}",
                self.index, key
            ));
        }
        Ok(())
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key export.routes[{}].{}", self.index, key))
    }

    fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid export.routes[{}].{}: expected true or false, got {value}",
                self.index, key
            )),
        }
    }

    fn required_parsed<T>(&self, key: &'static str) -> Result<T, String>
    where
        T: FromStr<Err = String>,
    {
        self.required(key)?.parse::<T>().map_err(|error| {
            format!(
                "invalid export route {} {}: {error}",
                self.values
                    .get("name")
                    .map(String::as_str)
                    .unwrap_or("<unnamed>"),
                key
            )
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AdapterSectionKey {
    kind: String,
    route_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Section {
    Root,
    Export,
    Route,
    Adapter { kind: String, route_name: String },
    Other,
}

fn parse_section_header(line: &str, line_number: usize) -> Result<Option<Section>, String> {
    if line.starts_with("[[") {
        if !line.ends_with("]]") {
            return Err(format!("invalid section header line {line_number}"));
        }
        if line == "[[export.routes]]" {
            return Ok(Some(Section::Route));
        }
        return Ok(Some(Section::Other));
    }
    if line.ends_with("]]") {
        return Err(format!("invalid section header line {line_number}"));
    }
    if !(line.starts_with('[') || line.ends_with(']')) {
        return Ok(None);
    }
    if !(line.starts_with('[') && line.ends_with(']')) {
        return Err(format!("invalid section header line {line_number}"));
    }
    let name = &line[1..line.len() - 1];
    if name == EXPORT_SECTION {
        return Ok(Some(Section::Export));
    }
    if let Some(adapter) = name.strip_prefix(EXPORT_ROUTE_ADAPTER_PREFIX) {
        let (kind, route_name) = adapter
            .split_once('.')
            .ok_or_else(|| format!("invalid export adapter section line {line_number}"))?;
        if kind.is_empty() || route_name.is_empty() {
            return Err(format!("invalid export adapter section line {line_number}"));
        }
        return Ok(Some(Section::Adapter {
            kind: kind.to_string(),
            route_name: route_name.to_string(),
        }));
    }
    Ok(Some(Section::Other))
}

fn parse_key_value(line: &str, line_number: usize) -> Result<Option<(String, String)>, String> {
    let Some((key, value)) = line.split_once('=') else {
        return Err(format!("invalid config line {line_number}"));
    };
    Ok(Some((key.trim().to_string(), unquote(value.trim())?)))
}

fn reject_legacy_key(line: &str) -> Result<(), String> {
    let Some((key, _)) = line.split_once('=') else {
        return Ok(());
    };
    let key = key.trim();
    if key.starts_with(LEGACY_OTEL_PREFIX) {
        return Err(format!(
            "unsupported config key {key}; use [export] and [[export.routes]]"
        ));
    }
    Ok(())
}

fn reject_export_key(section: &str, key: &str) -> Result<(), String> {
    if section == EXPORT_SECTION && key == "enabled" {
        return Ok(());
    }
    if section == EXPORT_SECTION {
        return Err(format!("unknown config key {section}.{key}"));
    }
    Ok(())
}

fn reject_route_key(key: &str) -> Result<(), String> {
    match key {
        "name" | "kind" | "delivery" | "enabled" => Ok(()),
        _ => Err(format!("unknown config key export.routes.{key}")),
    }
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(format!("invalid quoted value {value}"));
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}
