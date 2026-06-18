use std::collections::{BTreeMap, BTreeSet};

use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use recording_runtime::ObservedRecordWriteSession;

#[derive(Default)]
pub(super) struct RetainedPayloadTransaction {
    retained_bytes: BTreeMap<TraceId, u64>,
    persisted_traces: BTreeSet<TraceId>,
}

impl RetainedPayloadTransaction {
    pub(super) fn bytes(
        &mut self,
        cache: &mut BTreeMap<TraceId, u64>,
        session: &ObservedRecordWriteSession<'_>,
        trace_id: TraceId,
    ) -> Result<u64, ControlError> {
        if let Some(bytes) = self.retained_bytes.get(&trace_id) {
            return Ok(*bytes);
        }
        let bytes = match cache.get(&trace_id) {
            Some(bytes) => *bytes,
            None => {
                let bytes = session
                    .retained_payload_bytes(trace_id)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                cache.insert(trace_id, bytes);
                bytes
            }
        };
        self.retained_bytes.insert(trace_id, bytes);
        Ok(bytes)
    }

    pub(super) fn record_persisted(&mut self, trace_id: TraceId, retained_bytes: u64) {
        self.retained_bytes.insert(trace_id, retained_bytes);
        self.persisted_traces.insert(trace_id);
    }

    pub(super) fn apply_result<T, E>(
        self,
        cache: &mut BTreeMap<TraceId, u64>,
        result: &Result<T, E>,
    ) {
        if result.is_ok() {
            for trace_id in self.persisted_traces {
                if let Some(bytes) = self.retained_bytes.get(&trace_id) {
                    cache.insert(trace_id, *bytes);
                }
            }
        } else {
            for trace_id in self.persisted_traces {
                cache.remove(&trace_id);
            }
        }
    }
}
