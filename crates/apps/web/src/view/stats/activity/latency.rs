use std::collections::BTreeMap;

use crate::json;

use super::super::model::{
    BucketTotal, LatencyDistribution, LatencyGroupDistribution, LatencyGroupedSnapshot,
    LatencySnapshot, LatencyTrendSeries, LatencyTrendSnapshot, LlmUsageDataset, LlmUsageRow,
    Rollup,
};
use super::super::time_buckets::{bucket_points, bucket_start_ms};
use super::{Dimension, dimension_key_label};

pub(super) fn latency_snapshot(dataset: &LlmUsageDataset) -> LatencySnapshot {
    LatencySnapshot {
        ttft: latency_distribution(dataset, |row| row.latency.ttft_us),
        tpot: latency_distribution(dataset, |row| row.latency.tpot_us),
        grouped: LatencyGroupedSnapshot {
            models: latency_groups(dataset, Dimension::Model),
            endpoints: latency_groups(dataset, Dimension::Endpoint),
            apps: latency_groups(dataset, Dimension::App),
        },
        trends: LatencyTrendSnapshot {
            rollup: dataset.rollup,
            models: latency_trends(dataset, Dimension::Model),
            endpoints: latency_trends(dataset, Dimension::Endpoint),
            apps: latency_trends(dataset, Dimension::App),
        },
    }
}

pub(super) fn latency_snapshot_json(snapshot: &LatencySnapshot) -> String {
    format!(
        "{{\"ttft\":{},\"tpot\":{},\"grouped\":{},\"trends\":{}}}",
        latency_distribution_json(&snapshot.ttft),
        latency_distribution_json(&snapshot.tpot),
        latency_grouped_json(&snapshot.grouped),
        latency_trends_json(&snapshot.trends)
    )
}

fn latency_distribution(
    dataset: &LlmUsageDataset,
    sample: fn(&LlmUsageRow) -> Option<u64>,
) -> LatencyDistribution {
    let rows = dataset.rows.iter().collect::<Vec<_>>();
    latency_distribution_for_rows(&rows, sample)
}

fn latency_distribution_for_rows(
    rows: &[&LlmUsageRow],
    sample: fn(&LlmUsageRow) -> Option<u64>,
) -> LatencyDistribution {
    let mut samples = rows
        .iter()
        .filter_map(|row| sample(row))
        .collect::<Vec<_>>();
    samples.sort_unstable();
    let sample_count = samples.len();
    let missing_count = rows.len().saturating_sub(sample_count);
    LatencyDistribution {
        sample_count,
        missing_count,
        min_us: samples.first().copied(),
        max_us: samples.last().copied(),
        mean_us: mean(&samples),
        p50_us: percentile(&samples, 0.50),
        p90_us: percentile(&samples, 0.90),
        p95_us: percentile(&samples, 0.95),
        p99_us: percentile(&samples, 0.99),
        samples_us: samples,
    }
}

fn latency_groups(
    dataset: &LlmUsageDataset,
    dimension: Dimension,
) -> Vec<LatencyGroupDistribution> {
    grouped_rows(dataset, dimension)
        .into_iter()
        .filter_map(|group| {
            let ttft = latency_distribution_for_rows(&group.rows, |row| row.latency.ttft_us);
            let tpot = latency_distribution_for_rows(&group.rows, |row| row.latency.tpot_us);
            (ttft.sample_count + tpot.sample_count > 0).then(|| LatencyGroupDistribution {
                key: group.key,
                label: group.label,
                ttft,
                tpot,
            })
        })
        .collect()
}

fn latency_trends(dataset: &LlmUsageDataset, dimension: Dimension) -> Vec<LatencyTrendSeries> {
    grouped_rows(dataset, dimension)
        .into_iter()
        .filter_map(|group| {
            let ttft_avg =
                latency_avg_points(dataset.rollup, &group.rows, |row| row.latency.ttft_us);
            let tpot_avg =
                latency_avg_points(dataset.rollup, &group.rows, |row| row.latency.tpot_us);
            (ttft_avg.iter().any(|point| point.value > 0)
                || tpot_avg.iter().any(|point| point.value > 0))
            .then(|| LatencyTrendSeries {
                key: group.key,
                label: group.label,
                ttft_avg,
                tpot_avg,
            })
        })
        .collect()
}

fn latency_avg_points(
    rollup: Rollup,
    rows: &[&LlmUsageRow],
    sample: fn(&LlmUsageRow) -> Option<u64>,
) -> Vec<BucketTotal> {
    let mut buckets = BTreeMap::<u64, BucketAccumulator>::new();
    for row in rows {
        let Some(value) = sample(row) else {
            continue;
        };
        buckets
            .entry(bucket_start_ms(row.started_at_ms, rollup))
            .or_default()
            .add(value);
    }
    let values = buckets
        .into_iter()
        .map(|(bucket, accumulator)| (bucket, accumulator.mean()))
        .collect::<BTreeMap<_, _>>();
    bucket_points(rollup, values)
}

fn grouped_rows(dataset: &LlmUsageDataset, dimension: Dimension) -> Vec<GroupRows<'_>> {
    let mut groups = BTreeMap::<String, GroupRows<'_>>::new();
    for row in &dataset.rows {
        let Some((key, label)) = dimension_key_label(row, dimension) else {
            continue;
        };
        groups
            .entry(key.clone())
            .or_insert_with(|| GroupRows {
                key,
                label,
                rows: Vec::new(),
            })
            .rows
            .push(row);
    }
    let mut groups = groups.into_values().collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        latency_sample_count(&right.rows)
            .cmp(&latency_sample_count(&left.rows))
            .then_with(|| left.label.cmp(&right.label))
    });
    groups
}

fn latency_sample_count(rows: &[&LlmUsageRow]) -> usize {
    rows.iter()
        .filter(|row| row.latency.ttft_us.or(row.latency.tpot_us).is_some())
        .count()
}

struct GroupRows<'a> {
    key: String,
    label: String,
    rows: Vec<&'a LlmUsageRow>,
}

#[derive(Default)]
struct BucketAccumulator {
    total: u128,
    count: u64,
}

impl BucketAccumulator {
    fn add(&mut self, value: u64) {
        self.total += u128::from(value);
        self.count += 1;
    }

    fn mean(self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        u64::try_from(self.total / u128::from(self.count)).unwrap_or(u64::MAX)
    }
}

fn mean(samples: &[u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let total = samples
        .iter()
        .map(|sample| u128::from(*sample))
        .sum::<u128>();
    let count = u128::try_from(samples.len()).ok()?;
    Some(u64::try_from(total / count).unwrap_or(u64::MAX))
}

fn percentile(samples: &[u64], percentile: f64) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let index = ((samples.len() - 1) as f64 * percentile).round() as usize;
    samples.get(index).copied()
}

fn latency_distribution_json(distribution: &LatencyDistribution) -> String {
    format!(
        "{{\"sample_count\":{},\"missing_count\":{},\"min_us\":{},\"max_us\":{},\"mean_us\":{},\"p50_us\":{},\"p90_us\":{},\"p95_us\":{},\"p99_us\":{},\"samples_us\":{}}}",
        json::number(distribution.sample_count),
        json::number(distribution.missing_count),
        json::optional_number(distribution.min_us),
        json::optional_number(distribution.max_us),
        json::optional_number(distribution.mean_us),
        json::optional_number(distribution.p50_us),
        json::optional_number(distribution.p90_us),
        json::optional_number(distribution.p95_us),
        json::optional_number(distribution.p99_us),
        samples_json(&distribution.samples_us)
    )
}

fn latency_grouped_json(grouped: &LatencyGroupedSnapshot) -> String {
    format!(
        "{{\"models\":{},\"endpoints\":{},\"apps\":{}}}",
        latency_group_rows_json(&grouped.models),
        latency_group_rows_json(&grouped.endpoints),
        latency_group_rows_json(&grouped.apps)
    )
}

fn latency_group_rows_json(rows: &[LatencyGroupDistribution]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                format!(
                    "{{\"key\":{},\"label\":{},\"ttft\":{},\"tpot\":{}}}",
                    json::string(&row.key),
                    json::string(&row.label),
                    latency_distribution_json(&row.ttft),
                    latency_distribution_json(&row.tpot)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn latency_trends_json(trends: &LatencyTrendSnapshot) -> String {
    format!(
        "{{\"rollup\":{},\"models\":{},\"endpoints\":{},\"apps\":{}}}",
        json::string(trends.rollup.as_str()),
        latency_trend_rows_json(&trends.models),
        latency_trend_rows_json(&trends.endpoints),
        latency_trend_rows_json(&trends.apps)
    )
}

fn latency_trend_rows_json(rows: &[LatencyTrendSeries]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                format!(
                    "{{\"key\":{},\"label\":{},\"ttft_avg\":{},\"tpot_avg\":{}}}",
                    json::string(&row.key),
                    json::string(&row.label),
                    super::bucket_rows_json(&row.ttft_avg),
                    super::bucket_rows_json(&row.tpot_avg)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn samples_json(samples: &[u64]) -> String {
    format!(
        "[{}]",
        samples
            .iter()
            .map(json::number)
            .collect::<Vec<_>>()
            .join(",")
    )
}
