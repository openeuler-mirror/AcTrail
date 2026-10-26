use std::time::{Duration, UNIX_EPOCH};

use crate::command::OutputFormat;
use clap::Args;
use model_core::host_identity::HostBootId;
use resource_profiling::model::host_resource::HostResourceSample;
use resource_profiling::storage::database::{ResourceKind, ResourceQuery};

#[derive(Clone, Debug, Args)]
pub(crate) struct HostResourceArgs {
    #[arg(long)]
    boot_id: Option<HostBootId>,
    #[arg(long, help = "Inclusive Unix timestamp in seconds")]
    since: Option<u64>,
    #[arg(long, help = "Inclusive Unix timestamp in seconds")]
    until: Option<u64>,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
    limit: u32,
}

impl HostResourceArgs {
    pub(crate) fn query(&self) -> Result<ResourceQuery, String> {
        if self
            .since
            .zip(self.until)
            .is_some_and(|(since, until)| since > until)
        {
            return Err("--since must not exceed --until".into());
        }
        let timestamp = |value| {
            UNIX_EPOCH
                .checked_add(Duration::from_secs(value))
                .ok_or_else(|| "timestamp overflow".to_string())
        };
        Ok(ResourceQuery {
            kind: ResourceKind::Host,
            trace_id: None,
            host_boot_id: self.boot_id,
            since: self.since.map(timestamp).transpose()?,
            until: self.until.map(timestamp).transpose()?,
            limit: self.limit,
        })
    }
}

pub(crate) struct HostResourceView;

impl HostResourceView {
    pub(crate) fn render(
        samples: Vec<HostResourceSample>,
        format: OutputFormat,
    ) -> Result<String, String> {
        match format {
            OutputFormat::Json => {
                let rows = samples.iter().map(|sample| serde_json::json!({
                    "host_boot_id": sample.host_boot_id.to_string(),
                    "observed_at_unix_nanos": sample.observed_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().to_string(),
                    "cpu_percent_millis": sample.cpu_percent_millis,
                    "memory_total_bytes": sample.memory_total_bytes,
                    "memory_available_bytes": sample.memory_available_bytes,
                    "memory_free_bytes": sample.memory_free_bytes,
                    "load_average": sample.load_average,
                })).collect::<Vec<_>>();
                serde_json::to_string_pretty(&rows).map_err(|error| error.to_string())
            }
            OutputFormat::Table => {
                let mut output = String::from(
                    "BOOT_ID TIME_UNIX CPU_PERCENT MEM_TOTAL_BYTES MEM_AVAILABLE_BYTES MEM_FREE_BYTES LOAD_1M LOAD_5M LOAD_15M\n",
                );
                for sample in samples {
                    output.push_str(&format!(
                        "{} {} {} {} {} {} {} {} {}\n",
                        sample.host_boot_id,
                        sample
                            .observed_at
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        sample
                            .cpu_percent_millis
                            .map(|cpu| format!("{:.3}", cpu as f64 / 1000.0))
                            .unwrap_or_else(|| "unknown".into()),
                        sample.memory_total_bytes,
                        sample.memory_available_bytes,
                        sample.memory_free_bytes,
                        sample.load_average[0],
                        sample.load_average[1],
                        sample.load_average[2]
                    ));
                }
                Ok(output)
            }
        }
    }
}
