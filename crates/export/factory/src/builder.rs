use export_core::{ExportError, ExportRuntime};
use export_otel_jsonl::build_otel_jsonl_semantic_action_route;

use crate::{ExportConfig, ExportDeliveryConfig, ExportRouteTargetConfig};

pub fn build_export_runtime(config: &ExportConfig) -> Result<ExportRuntime, ExportError> {
    if !config.enabled {
        return Ok(ExportRuntime::new(Vec::new()));
    }
    if config.routes().iter().all(|route| !route.enabled) {
        return Err(ExportError::new(
            "export_factory",
            "export enabled but no enabled export routes configured",
        ));
    }
    let mut routes = Vec::new();
    for route in config.routes().iter().filter(|route| route.enabled) {
        route
            .target
            .validate_enabled_route()
            .map_err(|message| ExportError::new("export_factory", message))?;
        match route.delivery {
            ExportDeliveryConfig::BestEffort => match &route.target {
                ExportRouteTargetConfig::OtelJsonl(otel_jsonl) => {
                    routes.push(Box::new(build_otel_jsonl_semantic_action_route(
                        otel_jsonl.clone(),
                    )?)
                        as Box<dyn export_core::SemanticActionExportRoute>);
                }
            },
        }
    }
    Ok(ExportRuntime::new(routes))
}
