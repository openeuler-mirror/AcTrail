use crate::model::host_resource::HostResourceSample;
use model_core::host_identity::HostBootId;

use super::super::HostResourceProfilingBackend;
use super::host_counter_reader::HostCounterReader;
use crate::{ResourceProfilingBackend, SamplingContext};

pub struct LinuxHostResourceProfilingBackend {
    host_boot_id: HostBootId,
    reader: HostCounterReader,
    previous_cpu: Option<(u64, u64)>,
}

impl LinuxHostResourceProfilingBackend {
    pub fn new(host_boot_id: HostBootId) -> Self {
        Self {
            host_boot_id,
            reader: HostCounterReader,
            previous_cpu: None,
        }
    }
}

impl ResourceProfilingBackend for LinuxHostResourceProfilingBackend {
    type Input = ();
    type Sample = HostResourceSample;

    fn sample(&mut self, _: &(), context: SamplingContext) -> Result<Option<Self::Sample>, String> {
        let counters = self.reader.read()?;
        let previous = self
            .previous_cpu
            .replace((counters.busy_ticks, counters.total_ticks));
        let cpu_percent_millis = previous.and_then(|(busy, total)| {
            let elapsed = counters.total_ticks.checked_sub(total)?;
            let used = counters.busy_ticks.checked_sub(busy)?;
            if elapsed == 0 || used > elapsed {
                return None;
            }
            Some((u128::from(used) * 100_000 / u128::from(elapsed)) as u64)
        });
        Ok(Some(HostResourceSample {
            host_boot_id: self.host_boot_id,
            observed_at: context.observed_at,
            cpu_percent_millis,
            memory_total_bytes: counters.memory_total_bytes,
            memory_available_bytes: counters.memory_available_bytes,
            memory_free_bytes: counters.memory_free_bytes,
            load_average: counters.load_average,
            cpu_count: Some(counters.cpu_count).filter(|count| *count > 0),
        }))
    }
}

impl HostResourceProfilingBackend for LinuxHostResourceProfilingBackend {}
