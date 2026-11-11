//! Terminal output for daemon plugin commands.

use plugin_system::PluginInstanceStatus;

pub(super) struct PluginOutput;

impl PluginOutput {
    pub(super) fn list(items: &[PluginInstanceStatus]) {
        println!("INSTANCE\tPLUGIN\tPURPOSE\tRUNTIME\tSTATE\tHOST_GRANTS\tQUEUE\tWARNINGS");
        for item in items {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                item.instance_id,
                item.plugin_id,
                item.purpose.as_str(),
                item.runtime.as_str(),
                item.state.as_str(),
                Self::host_grants(&item.host_grants),
                Self::queue(item.queue_depth, item.queue_capacity),
                Self::warnings(&item.warnings)
            );
        }
    }

    pub(super) fn status(status: &PluginInstanceStatus) {
        println!("instance={}", status.instance_id);
        println!("plugin_id={}", status.plugin_id);
        println!("purpose={}", status.purpose.as_str());
        println!("runtime={}", status.runtime.as_str());
        println!("state={}", status.state.as_str());
        println!("host_grants={}", Self::host_grants(&status.host_grants));
        println!("queue_depth={}", Self::optional_u64(status.queue_depth));
        println!(
            "queue_capacity={}",
            Self::optional_u32(status.queue_capacity)
        );
        println!("observed_records={}", status.observed_records);
        println!("dropped_records={}", status.dropped_records);
        let payload_read = status.hostcall_metrics.payload_read;
        println!("payload_read_calls={}", payload_read.calls);
        println!("payload_read_bytes={}", payload_read.bytes);
        println!("payload_read_denied={}", payload_read.denied);
        println!("payload_read_not_found={}", payload_read.not_found);
        println!("payload_read_failed={}", payload_read.failed);
        println!("payload_read_invalid={}", payload_read.invalid);
        println!("payload_read_too_large={}", payload_read.too_large);
        println!("payload_read_truncated={}", payload_read.truncated);
        println!(
            "payload_read_latency_total_ns={}",
            payload_read.latency_total_ns
        );
        println!(
            "payload_read_latency_max_ns={}",
            payload_read.latency_max_ns
        );
        println!(
            "last_error={}",
            status.last_error.as_deref().unwrap_or("none")
        );
        for (name, value) in &status.operational_metrics {
            println!("metric.{name}={value}");
        }
        println!("warnings={}", Self::warnings(&status.warnings));
    }

    fn queue(queue_depth: Option<u64>, queue_capacity: Option<u32>) -> String {
        match (queue_depth, queue_capacity) {
            (Some(depth), Some(capacity)) => format!("{depth}/{capacity}"),
            _ => "none".to_string(),
        }
    }

    fn optional_u64(value: Option<u64>) -> String {
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    }

    fn optional_u32(value: Option<u32>) -> String {
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    }

    fn host_grants(host_grants: &[String]) -> String {
        if host_grants.is_empty() {
            "none".to_string()
        } else {
            host_grants.join(",")
        }
    }

    pub(super) fn warnings(warnings: &[String]) -> String {
        if warnings.is_empty() {
            "none".to_string()
        } else {
            warnings.join(";")
        }
    }
}
