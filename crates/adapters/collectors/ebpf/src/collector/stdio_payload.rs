//! Bounded assembly for stdio writes captured before syscall completion.

use std::collections::HashMap;

use collector_instance::CollectorError;
use collector_stats::DropCounter;
use model_core::ids::TraceId;
use model_core::payload::{PayloadOperationCompletionState, PayloadTruncationState};
use payload_event::RawPayloadSegment;

use crate::decode::{
    STDIO_PAYLOAD_DIRECTION_OUTBOUND, STDIO_PAYLOAD_FLAG_STAGED, STDIO_PAYLOAD_SYSCALL_WRITE,
    decode_stdio_payload,
};
use crate::loader::{KernelStdioPayloadCompletionEvent, KernelStdioPayloadEvent};
use crate::maps::BindingStateMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StdioOperationKey {
    trace_id: TraceId,
    host_pid: u32,
    host_tid: u32,
    pid_generation: u64,
}

impl StdioOperationKey {
    fn from_payload(event: &KernelStdioPayloadEvent) -> Self {
        Self {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            host_tid: event.host_tid,
            pid_generation: event.pid_generation,
        }
    }

    fn from_completion(event: &KernelStdioPayloadCompletionEvent) -> Self {
        Self {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            host_tid: event.host_tid,
            pid_generation: event.pid_generation,
        }
    }
}

struct StagedStdioPayload {
    event: KernelStdioPayloadEvent,
}

impl StagedStdioPayload {
    fn new(event: KernelStdioPayloadEvent) -> Self {
        Self { event }
    }

    fn matches(&self, completion: &KernelStdioPayloadCompletionEvent) -> bool {
        self.event.pid == completion.pid
            && self.event.tid == completion.tid
            && self.event.direction == completion.direction
            && self.event.sequence == completion.sequence
            && self.event.stream == completion.stream
            && self.event.fd == completion.fd
            && self.event.syscall == completion.syscall
    }

    fn complete(
        mut self,
        completed_size: u64,
        bindings: &BindingStateMap,
    ) -> Result<RawPayloadSegment, CollectorError> {
        let captured_size = completed_size.min(self.event.bytes.len() as u64);
        self.event.bytes.truncate(captured_size as usize);
        self.event.captured_size =
            u32::try_from(captured_size).expect("stdio capture limit must fit u32");
        let mut segment = decode_stdio_payload(self.event, bindings)
            .map_err(|error| CollectorError::new(error.stage, error.message))?;
        segment.original_size = completed_size;
        segment.captured_size = captured_size;
        segment.operation_original_size = completed_size;
        segment.operation_captured_size = captured_size;
        segment.operation_completion_state = PayloadOperationCompletionState::Success;
        segment.truncation = if captured_size == completed_size {
            PayloadTruncationState::Complete
        } else {
            PayloadTruncationState::Truncated
        };
        Ok(segment)
    }

    fn loss_marker(self, bindings: &BindingStateMap) -> Result<RawPayloadSegment, CollectorError> {
        let original_size = u64::from(self.event.original_size);
        Self::decode_loss_marker(self.event, original_size, bindings)
    }

    fn completion_loss_marker(
        completion: KernelStdioPayloadCompletionEvent,
        bindings: &BindingStateMap,
    ) -> Result<RawPayloadSegment, CollectorError> {
        let original_size = completion.requested_size;
        let event = KernelStdioPayloadEvent {
            pid: completion.pid,
            tid: completion.tid,
            host_pid: completion.host_pid,
            host_tid: completion.host_tid,
            direction: completion.direction,
            trace_id: completion.trace_id,
            observed_ktime_ns: completion.observed_ktime_ns,
            sequence: completion.sequence,
            stream: completion.stream,
            original_size: 0,
            captured_size: 0,
            flags: 0,
            fd: completion.fd,
            syscall: completion.syscall,
            pid_generation: completion.pid_generation,
            bytes: Vec::new(),
        };
        Self::decode_loss_marker(event, original_size, bindings)
    }

    fn decode_loss_marker(
        mut event: KernelStdioPayloadEvent,
        original_size: u64,
        bindings: &BindingStateMap,
    ) -> Result<RawPayloadSegment, CollectorError> {
        event.captured_size = 0;
        event.bytes.clear();
        let mut segment = decode_stdio_payload(event, bindings)
            .map_err(|error| CollectorError::new(error.stage, error.message))?;
        segment.original_size = original_size;
        segment.captured_size = 0;
        segment.operation_original_size = original_size;
        segment.operation_captured_size = 0;
        segment.operation_completion_state = PayloadOperationCompletionState::Partial;
        segment.truncation = PayloadTruncationState::Truncated;
        Ok(segment)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AssemblyLossCounters {
    capacity_exhausted: u64,
    duplicate_stage: u64,
    completion_without_stage: u64,
    completion_mismatch: u64,
    abandoned_on_process_exit: u64,
    abandoned_on_trace_release: u64,
}

impl AssemblyLossCounters {
    fn total(self) -> u64 {
        self.capacity_exhausted
            .saturating_add(self.duplicate_stage)
            .saturating_add(self.completion_without_stage)
            .saturating_add(self.completion_mismatch)
            .saturating_add(self.abandoned_on_process_exit)
            .saturating_add(self.abandoned_on_trace_release)
    }

    fn summary(self) -> String {
        format!(
            "stdio payload assembly lost data: capacity_exhausted={}, duplicate_stage={}, completion_without_stage={}, completion_mismatch={}, abandoned_on_process_exit={}, abandoned_on_trace_release={}",
            self.capacity_exhausted,
            self.duplicate_stage,
            self.completion_without_stage,
            self.completion_mismatch,
            self.abandoned_on_process_exit,
            self.abandoned_on_trace_release,
        )
    }

    fn append_drop_counters(self, dropped: &mut Vec<DropCounter>) {
        for (reason, count) in [
            ("capacity_exhausted", self.capacity_exhausted),
            ("duplicate_stage", self.duplicate_stage),
            ("completion_without_stage", self.completion_without_stage),
            ("completion_mismatch", self.completion_mismatch),
            ("abandoned_on_process_exit", self.abandoned_on_process_exit),
            (
                "abandoned_on_trace_release",
                self.abandoned_on_trace_release,
            ),
        ] {
            if count != 0 {
                dropped.push(DropCounter {
                    reason: format!("ebpf_stdio_payload_assembly_loss:{reason}"),
                    count,
                });
            }
        }
    }
}

pub(super) struct StdioPayloadAssembler {
    pending: HashMap<StdioOperationKey, StagedStdioPayload>,
    max_pending: usize,
    losses: AssemblyLossCounters,
    reported_losses: AssemblyLossCounters,
}

impl StdioPayloadAssembler {
    pub(super) fn new(max_pending: u32) -> Self {
        assert!(
            max_pending != 0,
            "stdio payload pending-operation limit must be positive"
        );
        Self {
            pending: HashMap::new(),
            max_pending: max_pending as usize,
            losses: AssemblyLossCounters::default(),
            reported_losses: AssemblyLossCounters::default(),
        }
    }

    pub(super) fn observe_payload(
        &mut self,
        event: KernelStdioPayloadEvent,
        bindings: &BindingStateMap,
    ) -> Result<Option<RawPayloadSegment>, CollectorError> {
        if event.flags & STDIO_PAYLOAD_FLAG_STAGED == 0 {
            return decode_stdio_payload(event, bindings)
                .map(Some)
                .map_err(|error| CollectorError::new(error.stage, error.message));
        }
        if event.direction != STDIO_PAYLOAD_DIRECTION_OUTBOUND
            || event.syscall != STDIO_PAYLOAD_SYSCALL_WRITE
        {
            return Err(CollectorError::new(
                "stdio_payload_assembly",
                "staged stdio payload is not an outbound write",
            ));
        }

        let key = StdioOperationKey::from_payload(&event);
        let staged = StagedStdioPayload::new(event);
        let at_capacity = self.pending.len() >= self.max_pending;
        match self.pending.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                self.losses.duplicate_stage = self.losses.duplicate_stage.saturating_add(1);
                let previous = entry.insert(staged);
                previous.loss_marker(bindings).map(Some)
            }
            std::collections::hash_map::Entry::Vacant(_) if at_capacity => {
                self.losses.capacity_exhausted = self.losses.capacity_exhausted.saturating_add(1);
                staged.loss_marker(bindings).map(Some)
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(staged);
                Ok(None)
            }
        }
    }

    pub(super) fn observe_completion(
        &mut self,
        completion: KernelStdioPayloadCompletionEvent,
        bindings: &BindingStateMap,
    ) -> Result<Option<RawPayloadSegment>, CollectorError> {
        let key = StdioOperationKey::from_completion(&completion);
        let Some(staged) = self.pending.remove(&key) else {
            if completion.result <= 0 {
                return Ok(None);
            }
            self.losses.completion_without_stage =
                self.losses.completion_without_stage.saturating_add(1);
            return StagedStdioPayload::completion_loss_marker(completion, bindings).map(Some);
        };
        if !staged.matches(&completion)
            || completion.result > 0 && completion.result as u64 > completion.requested_size
        {
            self.losses.completion_mismatch = self.losses.completion_mismatch.saturating_add(1);
            return staged.loss_marker(bindings).map(Some);
        }
        if completion.result <= 0 {
            return Ok(None);
        }
        staged
            .complete(completion.result as u64, bindings)
            .map(Some)
    }

    pub(super) fn release_process(
        &mut self,
        trace_id: TraceId,
        host_pid: u32,
        pid_generation: u64,
    ) {
        let previous_len = self.pending.len();
        self.pending.retain(|key, _| {
            key.trace_id != trace_id
                || key.host_pid != host_pid
                || key.pid_generation != pid_generation
        });
        let removed = previous_len.saturating_sub(self.pending.len()) as u64;
        self.losses.abandoned_on_process_exit = self
            .losses
            .abandoned_on_process_exit
            .saturating_add(removed);
    }

    pub(super) fn release_trace(&mut self, trace_id: TraceId) {
        let previous_len = self.pending.len();
        self.pending.retain(|key, _| key.trace_id != trace_id);
        let removed = previous_len.saturating_sub(self.pending.len()) as u64;
        self.losses.abandoned_on_trace_release = self
            .losses
            .abandoned_on_trace_release
            .saturating_add(removed);
    }

    pub(super) fn append_drop_counters(&self, dropped: &mut Vec<DropCounter>) {
        self.losses.append_drop_counters(dropped);
    }

    pub(super) fn take_loss_summaries(&mut self) -> Vec<String> {
        if self.losses == self.reported_losses || self.losses.total() == 0 {
            return Vec::new();
        }
        self.reported_losses = self.losses;
        vec![self.losses.summary()]
    }
}
