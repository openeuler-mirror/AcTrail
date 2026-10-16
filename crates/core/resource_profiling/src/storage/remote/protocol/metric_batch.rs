use crate::{model::ResourceSample, storage::ResourceStorageError};
use std::{collections::BTreeMap, time::UNIX_EPOCH};
pub(super) struct MetricPoint {
    pub name: String,
    pub value: f64,
    pub timestamp_ns: i64,
    pub labels: BTreeMap<String, String>,
}
pub(super) struct MetricBatch;
impl MetricBatch {
    pub(super) fn points(
        samples: &[ResourceSample],
    ) -> Result<Vec<MetricPoint>, ResourceStorageError> {
        let mut points = Vec::new();
        for sample in samples {
            let timestamp_ns = sample
                .observed_at()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_nanos()).ok())
                .ok_or_else(|| {
                    ResourceStorageError("resource timestamp outside remote range".into())
                })?;
            let mut labels = BTreeMap::new();
            let value = match sample {
                ResourceSample::Host(s) => {
                    labels.insert("kind".into(), "host".into());
                    labels.insert("host_boot_id".into(), s.host_boot_id.to_string());
                    let mut value = serde_json::to_value(s)?;
                    if let Some(obj) = value.as_object_mut() {
                        obj.remove("observed_at");
                        obj.remove("host_boot_id");
                        obj.remove("load_average");
                        for (suffix, load) in ["1m", "5m", "15m"].iter().zip(s.load_average) {
                            obj.insert(format!("load_average_{suffix}"), serde_json::json!(load));
                        }
                    }
                    value
                }
                ResourceSample::Workload(s) => {
                    labels.insert("kind".into(), "workload".into());
                    labels.insert("trace_id".into(), s.trace_id.get().to_string());
                    labels.insert("process_id".into(), s.process.get().to_string());
                    labels.insert(
                        "accounting_method".into(),
                        s.payload.accounting_method.as_str().into(),
                    );
                    labels.insert(
                        "accounting_coverage".into(),
                        s.payload.accounting_coverage.as_str().into(),
                    );
                    labels.insert("sample_kind".into(), s.payload.sample_kind.as_str().into());
                    let mut value = serde_json::to_value(&s.payload)?;
                    value
                        .as_object_mut()
                        .expect("resource struct serializes as object")
                        .remove("metadata");
                    value
                }
            };
            Self::fields(
                "actrail_resource",
                &value,
                timestamp_ns,
                &labels,
                &mut points,
            );
        }
        Ok(points)
    }
    fn fields(
        prefix: &str,
        value: &serde_json::Value,
        timestamp_ns: i64,
        labels: &BTreeMap<String, String>,
        out: &mut Vec<MetricPoint>,
    ) {
        if let Some(fields) = value.as_object() {
            for (name, value) in fields {
                Self::fields(
                    &format!("{prefix}_{name}"),
                    value,
                    timestamp_ns,
                    labels,
                    out,
                );
            }
        } else if let Some(value) = value.as_f64().filter(|v| v.is_finite()) {
            out.push(MetricPoint {
                name: prefix.into(),
                value,
                timestamp_ns,
                labels: labels.clone(),
            });
        }
    }
}
