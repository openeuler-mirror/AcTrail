use super::metric_batch::MetricBatch;
use crate::{model::ResourceSample, storage::ResourceStorageError};
pub(in crate::storage::remote) struct InfluxdbLineProtocolClient;
impl InfluxdbLineProtocolClient {
    pub(in crate::storage::remote) fn encode(
        samples: &[ResourceSample],
    ) -> Result<String, ResourceStorageError> {
        let mut body = String::new();
        for point in MetricBatch::points(samples)? {
            body.push_str(&point.name);
            for (key, value) in point.labels {
                body.push(',');
                body.push_str(&key);
                body.push('=');
                body.push_str(&Self::escape(&value));
            }
            body.push_str(&format!(" value={} {}\n", point.value, point.timestamp_ns));
        }
        Ok(body)
    }
    fn escape(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace(',', "\\,")
            .replace(' ', "\\ ")
            .replace('=', "\\=")
    }
}
