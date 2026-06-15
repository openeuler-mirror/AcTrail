use std::path::PathBuf;
use std::str::FromStr;

use export_otel_jsonl::OtelJsonlExporterConfig;

use crate::parser::parse_export_config;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportConfig {
    pub enabled: bool,
    routes: Vec<ExportRouteConfig>,
}

impl ExportConfig {
    pub fn new(enabled: bool, routes: Vec<ExportRouteConfig>) -> Self {
        Self { enabled, routes }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_export_config(raw)
    }

    pub fn routes(&self) -> &[ExportRouteConfig] {
        &self.routes
    }

    pub fn enabled_output_files(&self) -> Vec<ExportOutputFile> {
        if !self.enabled {
            return Vec::new();
        }
        self.routes
            .iter()
            .filter(|route| route.enabled)
            .map(|route| match &route.target {
                ExportRouteTargetConfig::OtelJsonl(config) => ExportOutputFile {
                    label: "export_otel_jsonl_path",
                    route_name: route.name.clone(),
                    path: config.path.clone(),
                },
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportRouteConfig {
    pub name: String,
    pub enabled: bool,
    pub delivery: ExportDeliveryConfig,
    pub target: ExportRouteTargetConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExportRouteTargetConfig {
    OtelJsonl(OtelJsonlExporterConfig),
}

impl ExportRouteTargetConfig {
    pub const fn kind(&self) -> ExportRouteKind {
        match self {
            Self::OtelJsonl(_) => ExportRouteKind::OtelJsonl,
        }
    }

    pub fn validate_enabled_route(&self) -> Result<(), String> {
        match self {
            Self::OtelJsonl(config) => config.validate_enabled_route(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportRouteKind {
    OtelJsonl,
}

impl ExportRouteKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OtelJsonl => "otel-jsonl",
        }
    }
}

impl FromStr for ExportRouteKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "otel-jsonl" => Ok(Self::OtelJsonl),
            _ => Err("expected otel-jsonl".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportDeliveryConfig {
    BestEffort,
}

impl ExportDeliveryConfig {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BestEffort => "best-effort",
        }
    }
}

impl FromStr for ExportDeliveryConfig {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "best-effort" => Ok(Self::BestEffort),
            _ => Err("expected best-effort".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportOutputFile {
    pub label: &'static str,
    pub route_name: String,
    pub path: PathBuf,
}
