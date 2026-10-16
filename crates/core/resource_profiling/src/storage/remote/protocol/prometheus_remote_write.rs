use super::metric_batch::MetricBatch;
use crate::{model::ResourceSample, storage::ResourceStorageError};
use prost::Message;
#[derive(Clone, PartialEq, Message)]
struct Label {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(string, tag = "2")]
    value: String,
}
#[derive(Clone, PartialEq, Message)]
struct Sample {
    #[prost(double, tag = "1")]
    value: f64,
    #[prost(int64, tag = "2")]
    timestamp: i64,
}
#[derive(Clone, PartialEq, Message)]
struct TimeSeries {
    #[prost(message, repeated, tag = "1")]
    labels: Vec<Label>,
    #[prost(message, repeated, tag = "2")]
    samples: Vec<Sample>,
}
#[derive(Clone, PartialEq, Message)]
struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    timeseries: Vec<TimeSeries>,
}
pub(in crate::storage::remote) struct PrometheusRemoteWriteClient;
impl PrometheusRemoteWriteClient {
    pub(in crate::storage::remote) fn encode(
        samples: &[ResourceSample],
    ) -> Result<Vec<u8>, ResourceStorageError> {
        let mut series: std::collections::BTreeMap<Vec<(String, String)>, Vec<Sample>> =
            Default::default();
        for point in MetricBatch::points(samples)? {
            let mut labels = point.labels;
            labels.insert("__name__".into(), point.name);
            series
                .entry(labels.into_iter().collect())
                .or_default()
                .push(Sample {
                    value: point.value,
                    timestamp: point.timestamp_ns / 1_000_000,
                });
        }
        let timeseries = series
            .into_iter()
            .map(|(labels, mut samples)| {
                samples.sort_by_key(|sample| sample.timestamp);
                samples.dedup_by_key(|sample| sample.timestamp);
                TimeSeries {
                    labels: labels
                        .into_iter()
                        .map(|(name, value)| Label { name, value })
                        .collect(),
                    samples,
                }
            })
            .collect();
        snap::raw::Encoder::new()
            .compress_vec(&WriteRequest { timeseries }.encode_to_vec())
            .map_err(|e| ResourceStorageError(e.to_string()))
    }
}
