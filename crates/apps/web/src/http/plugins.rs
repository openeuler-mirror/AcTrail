use std::path::Path;

use config_core::daemon::OperatorConfig;
use serde::Deserialize;

use super::query::required_query_param;
use super::{Response, STATUS_BAD_REQUEST, STATUS_METHOD_NOT_ALLOWED};
use crate::plugins::{
    InstalledPluginCatalog, PluginLoadOptions, catalog_json, plugin_command_json,
    plugin_config_json, plugin_config_validation_json, plugin_status_json,
    unavailable_catalog_json,
};

#[derive(Deserialize)]
struct PluginCommandBody {
    argv: Vec<String>,
}

#[derive(Deserialize)]
struct PluginConfigBody {
    config: serde_json::Value,
}

pub(super) struct PluginHttp<'a> {
    config_path: Option<&'a Path>,
    config: Option<&'a OperatorConfig>,
}

impl<'a> PluginHttp<'a> {
    pub(super) fn new(config_path: Option<&'a Path>, config: Option<&'a OperatorConfig>) -> Self {
        Self {
            config_path,
            config,
        }
    }

    pub(super) fn route(
        &self,
        method: &str,
        path: &str,
        query: &str,
        body: &[u8],
    ) -> Option<Result<Response, String>> {
        match path {
            "/api/plugins/catalog" => Some(self.refresh(method)),
            "/api/plugins/catalog/load" => Some(self.load(method, query, body)),
            "/api/plugins/runtime/unload" => Some(self.unload(method, query)),
            "/api/plugins/runtime/command" => Some(self.command(method, query, body)),
            "/api/plugins/runtime/config" => Some(self.config(method, query, body)),
            "/api/plugins/runtime/config/validate" => {
                Some(self.validate_config(method, query, body))
            }
            _ => None,
        }
    }

    fn refresh(&self, method: &str) -> Result<Response, String> {
        if method != "GET" {
            return Ok(Self::method_not_allowed("GET", "/api/plugins/catalog"));
        }
        let Some(catalog) = self.catalog()? else {
            return Ok(Response::json(unavailable_catalog_json()));
        };
        let snapshot = catalog.refresh()?;
        catalog_json(&snapshot).map(Response::json)
    }

    fn load(&self, method: &str, query: &str, body: &[u8]) -> Result<Response, String> {
        if method != "POST" {
            return Ok(Self::method_not_allowed(
                "POST",
                "/api/plugins/catalog/load",
            ));
        }
        self.required_catalog()
            .and_then(|catalog| {
                let package_key = required_query_param(query, "package")?;
                let options = serde_json::from_slice::<PluginLoadOptions>(body)
                    .map_err(|error| format!("invalid plugin load options JSON: {error}"))?;
                catalog.load(&package_key, options)
            })
            .and_then(|status| plugin_status_json(&status))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
    }

    fn unload(&self, method: &str, query: &str) -> Result<Response, String> {
        if method != "POST" {
            return Ok(Self::method_not_allowed(
                "POST",
                "/api/plugins/runtime/unload",
            ));
        }
        self.required_catalog()
            .and_then(|catalog| {
                required_query_param(query, "instance_id")
                    .and_then(|instance_id| catalog.unload(&instance_id))
            })
            .and_then(|status| plugin_status_json(&status))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
    }

    fn command(&self, method: &str, query: &str, body: &[u8]) -> Result<Response, String> {
        if method != "POST" {
            return Ok(Self::method_not_allowed(
                "POST",
                "/api/plugins/runtime/command",
            ));
        }
        self.required_catalog()
            .and_then(|catalog| {
                let instance_id = required_query_param(query, "instance_id")?;
                let body = std::str::from_utf8(body)
                    .map_err(|error| format!("invalid UTF-8 plugin command body: {error}"))?;
                let command: PluginCommandBody = serde_json::from_str(body)
                    .map_err(|error| format!("invalid plugin command JSON body: {error}"))?;
                catalog.command(&instance_id, command.argv)
            })
            .and_then(|reply| plugin_command_json(&reply))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
    }

    fn config(&self, method: &str, query: &str, body: &[u8]) -> Result<Response, String> {
        match method {
            "GET" => self
                .required_catalog()
                .and_then(|catalog| {
                    required_query_param(query, "instance_id")
                        .and_then(|instance_id| catalog.config(&instance_id))
                })
                .and_then(|reply| plugin_config_json(&reply))
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
            "POST" => self
                .required_catalog()
                .and_then(|catalog| {
                    let instance_id = required_query_param(query, "instance_id")?;
                    Self::config_json(body)
                        .and_then(|config_json| catalog.update_config(&instance_id, config_json))
                })
                .and_then(|reply| plugin_config_json(&reply))
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
            _ => Ok(Self::method_not_allowed(
                "GET or POST",
                "/api/plugins/runtime/config",
            )),
        }
    }

    fn validate_config(&self, method: &str, query: &str, body: &[u8]) -> Result<Response, String> {
        if method != "POST" {
            return Ok(Self::method_not_allowed(
                "POST",
                "/api/plugins/runtime/config/validate",
            ));
        }
        self.required_catalog()
            .and_then(|catalog| {
                let instance_id = required_query_param(query, "instance_id")?;
                Self::config_json(body)
                    .and_then(|config_json| catalog.validate_config(&instance_id, config_json))
            })
            .and_then(|reply| plugin_config_validation_json(&reply))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
    }

    fn config_json(body: &[u8]) -> Result<String, String> {
        let body = std::str::from_utf8(body)
            .map_err(|error| format!("invalid UTF-8 plugin config body: {error}"))?;
        let request: PluginConfigBody = serde_json::from_str(body)
            .map_err(|error| format!("invalid plugin config JSON body: {error}"))?;
        serde_json::to_string(&request.config)
            .map_err(|error| format!("serialize plugin config request failed: {error}"))
    }

    fn required_catalog(&self) -> Result<InstalledPluginCatalog, String> {
        self.catalog()?.ok_or_else(|| {
            "operator config was not loaded; plugin control is unavailable in storage-only mode"
                .to_string()
        })
    }

    fn catalog(&self) -> Result<Option<InstalledPluginCatalog>, String> {
        self.config
            .map(|config| InstalledPluginCatalog::new(self.config_path, config))
            .transpose()
    }

    fn method_not_allowed(method: &str, path: &str) -> Response {
        Response::text(
            STATUS_METHOD_NOT_ALLOWED,
            format!("{method} required for {path}"),
        )
    }
}
