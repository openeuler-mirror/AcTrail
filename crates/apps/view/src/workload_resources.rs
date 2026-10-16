use crate::command::OutputFormat;
use resource_profiling::model::WorkloadResourceSample;
pub(crate) struct WorkloadResourceView;
impl WorkloadResourceView {
    pub(crate) fn render(
        samples: Vec<WorkloadResourceSample>,
        format: OutputFormat,
    ) -> Result<String, String> {
        match format {
            OutputFormat::Json => serde_json::to_string_pretty(&samples).map_err(|e| e.to_string()),
            OutputFormat::Table => {
                let mut output = String::from(
                    "TRACE PROCESS TIME_UNIX SOURCE COVERAGE KIND CPU_PERCENT RSS_KB CGROUP_MEMORY_BYTES\n",
                );
                for sample in samples {
                    output.push_str(&format!(
                        "{} {} {} {} {} {} {} {} {}\n",
                        sample.trace_id,
                        sample.process,
                        sample
                            .observed_at
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        sample.payload.accounting_method.as_str(),
                        sample.payload.accounting_coverage.as_str(),
                        sample.payload.sample_kind.as_str(),
                        sample
                            .payload
                            .cpu_percent_millis
                            .map(|v| format!("{:.3}", v as f64 / 1000.0))
                            .unwrap_or_else(|| "unknown".into()),
                        sample
                            .payload
                            .rss_kb
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        sample
                            .payload
                            .memory_current_bytes
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    ));
                }
                Ok(output)
            }
        }
    }
}
