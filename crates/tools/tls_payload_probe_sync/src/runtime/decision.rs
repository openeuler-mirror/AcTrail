//! Shared synchronous payload decision execution.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use tls_payload_core::{Decision, PayloadDirection};
use tls_payload_sync::{DecisionEvent, PayloadEvent, SummaryEvent, SyncEvent};

use crate::runtime::flow_control::{FlowDecision, FlowEmission, FlowSummary};
use crate::runtime::{config, output};

pub(super) enum RuntimeAction {
    Allow,
    Replace(Vec<u8>),
    Block,
}

pub(super) fn decide_payload(
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    payload: &[u8],
) -> RuntimeAction {
    let Some(config) = config::get() else {
        return RuntimeAction::Allow;
    };
    let sequence = config.next_sequence();
    if payload.len() > config.max_payload_bytes() {
        report_decision(
            "block",
            direction,
            symbol,
            stream_key,
            sequence,
            payload.len(),
            "max_payload_bytes",
        );
        return RuntimeAction::Block;
    }
    let provider = config.provider_for_symbol(symbol);
    report_payload(direction, symbol, stream_key, sequence, payload);
    match config.decide(&provider, symbol, direction, stream_key, payload) {
        Ok(Decision::Allow) => RuntimeAction::Allow,
        Ok(Decision::Block { reason }) => {
            report_decision(
                "block",
                direction,
                symbol,
                stream_key,
                sequence,
                payload.len(),
                &reason,
            );
            RuntimeAction::Block
        }
        Ok(Decision::ReplaceEqualLen {
            replacement,
            reason,
        }) => {
            if replacement.len() != payload.len() {
                report_decision(
                    "block",
                    direction,
                    symbol,
                    stream_key,
                    sequence,
                    payload.len(),
                    "replacement_len_mismatch",
                );
                return RuntimeAction::Block;
            }
            report_decision(
                "replace_equal_len",
                direction,
                symbol,
                stream_key,
                sequence,
                payload.len(),
                &reason,
            );
            RuntimeAction::Replace(replacement)
        }
        Err(error) => {
            report_decision(
                "block",
                direction,
                symbol,
                stream_key,
                sequence,
                payload.len(),
                &format!("processor_error:{error}"),
            );
            RuntimeAction::Block
        }
    }
}

pub(super) fn report_payload(
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    payload: &[u8],
) {
    let Some(config) = config::get() else {
        return;
    };
    let provider = config.provider_for_symbol(symbol);
    let flow_decision = config.classify_flow(direction, stream_key, payload);
    match &flow_decision {
        FlowDecision::EmitPayload => {
            send_payload_event(
                config, &provider, direction, symbol, stream_key, sequence, payload,
            );
        }
        FlowDecision::EmitSummary(summary) => {
            send_summary_event(
                config,
                &provider,
                direction,
                symbol,
                stream_key,
                sequence,
                summary.clone(),
            );
        }
        FlowDecision::EmitMany(emissions) => {
            for emission in emissions {
                send_flow_emission(
                    config, &provider, direction, symbol, stream_key, sequence, emission,
                );
            }
        }
        FlowDecision::DropBody => {}
    }
    if !config.should_print_payload() {
        return;
    }
    match flow_decision {
        FlowDecision::EmitPayload => output::event_line(&format!(
            "sync_payload: direction={} provider={} symbol={} stream=0x{:x} bytes={} preview={}\n",
            direction.as_str(),
            provider,
            symbol,
            stream_key,
            payload.len(),
            config.redact_payload(payload)
        )),
        FlowDecision::EmitSummary(summary) => output::event_line(&format!(
            "sync_payload_summary: direction={} provider={} symbol={} stream=0x{:x} observed_bytes={}\n",
            direction.as_str(),
            provider,
            symbol,
            stream_key,
            summary.observed_size
        )),
        FlowDecision::EmitMany(emissions) => {
            for emission in emissions {
                match emission {
                    FlowEmission::Payload(bytes) => output::event_line(&format!(
                        "sync_payload: direction={} provider={} symbol={} stream=0x{:x} bytes={} preview={}\n",
                        direction.as_str(),
                        provider,
                        symbol,
                        stream_key,
                        bytes.len(),
                        config.redact_payload(&bytes)
                    )),
                    FlowEmission::Summary(summary) => output::event_line(&format!(
                        "sync_payload_summary: direction={} provider={} symbol={} stream=0x{:x} observed_bytes={}\n",
                        direction.as_str(),
                        provider,
                        symbol,
                        stream_key,
                        summary.observed_size
                    )),
                }
            }
        }
        FlowDecision::DropBody => output::event_line(&format!(
            "sync_payload_drop_body: direction={} provider={} symbol={} stream=0x{:x} bytes={}\n",
            direction.as_str(),
            provider,
            symbol,
            stream_key,
            payload.len()
        )),
    }
}

fn send_flow_emission(
    config: &config::RuntimeConfig,
    provider: &str,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    emission: &FlowEmission,
) {
    match emission {
        FlowEmission::Payload(bytes) => {
            send_payload_event(
                config, provider, direction, symbol, stream_key, sequence, bytes,
            );
        }
        FlowEmission::Summary(summary) => {
            send_summary_event(
                config,
                provider,
                direction,
                symbol,
                stream_key,
                sequence,
                summary.clone(),
            );
        }
    }
}

fn send_summary_event(
    config: &config::RuntimeConfig,
    provider: &str,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    summary: FlowSummary,
) {
    let Some(trace_id) = config.trace_id() else {
        return;
    };
    let Some((pid, start_time_ticks, pid_namespace)) = PROCESS_METADATA_CACHE.current() else {
        return;
    };
    let event = SyncEvent::Summary(SummaryEvent {
        trace_id,
        pid,
        start_time_ticks,
        pid_namespace,
        direction,
        provider: provider.to_string(),
        symbol: symbol.to_string(),
        stream_key: stream_key as u64,
        sequence,
        observed_size: summary.observed_size,
        emitted_size: summary.bytes.len() as u64,
        reason: summary.reason.to_string(),
        protocol_hint: summary.protocol_hint.to_string(),
        bytes: summary.bytes,
    });
    send_event_or_drop(config, event);
}

pub(super) fn report_decision(
    action: &str,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    bytes: usize,
    reason: &str,
) {
    let Some(config) = config::get() else {
        return;
    };
    let provider = config.provider_for_symbol(symbol);
    send_decision_event(
        config, &provider, action, direction, symbol, stream_key, sequence, reason,
    );
    if !config.should_print_decision() {
        return;
    }
    output::event_line(&format!(
        "sync_decision: action={action} direction={} provider={} symbol={symbol} bytes={bytes} reason={reason}\n",
        direction.as_str(),
        provider,
    ));
}

fn send_payload_event(
    config: &config::RuntimeConfig,
    provider: &str,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    payload: &[u8],
) {
    let Some(trace_id) = config.trace_id() else {
        return;
    };
    let Some((pid, start_time_ticks, pid_namespace)) = PROCESS_METADATA_CACHE.current() else {
        return;
    };
    let event = SyncEvent::Payload(PayloadEvent {
        trace_id,
        pid,
        start_time_ticks,
        pid_namespace,
        direction,
        provider: provider.to_string(),
        symbol: symbol.to_string(),
        stream_key: stream_key as u64,
        sequence,
        bytes: payload.to_vec(),
    });
    send_event_or_drop(config, event);
}

fn send_decision_event(
    config: &config::RuntimeConfig,
    provider: &str,
    action: &str,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    reason: &str,
) {
    let Some(trace_id) = config.trace_id() else {
        return;
    };
    let Some((pid, start_time_ticks, pid_namespace)) = PROCESS_METADATA_CACHE.current() else {
        return;
    };
    let event = SyncEvent::Decision(DecisionEvent {
        trace_id,
        pid,
        start_time_ticks,
        pid_namespace,
        direction,
        provider: provider.to_string(),
        symbol: symbol.to_string(),
        stream_key: stream_key as u64,
        sequence,
        action: action.to_string(),
        reason: reason.to_string(),
    });
    send_event_or_drop(config, event);
}

fn send_event_or_drop(config: &config::RuntimeConfig, event: SyncEvent) {
    if let Err(error) = config.send_event(event) {
        if config.should_print_decision() {
            output::error_line(&format!("tls_payload_probe_sync event dropped: {error}\n"));
        }
    }
}

struct ProcessMetadataCache {
    pid: AtomicU32,
    start_ticks: AtomicU64,
    namespace_inode: AtomicU64,
}

static PROCESS_METADATA_CACHE: ProcessMetadataCache = ProcessMetadataCache {
    pid: AtomicU32::new(0),
    start_ticks: AtomicU64::new(0),
    namespace_inode: AtomicU64::new(0),
};

impl ProcessMetadataCache {
    fn current(&self) -> Option<(u32, u64, String)> {
        let pid = unsafe { libc::getpid() as u32 };
        if self.pid.load(Ordering::Acquire) == pid {
            let start_time_ticks = self.start_ticks.load(Ordering::Relaxed);
            let namespace_inode = self.namespace_inode.load(Ordering::Relaxed);
            if start_time_ticks != 0 && namespace_inode != 0 {
                return Some((pid, start_time_ticks, format!("pid:[{namespace_inode}]")));
            }
        }

        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let fields = stat.rsplit_once(") ")?.1.split_whitespace();
        let start_time_ticks = fields.skip(19).next()?.parse().ok()?;
        let pid_namespace = std::fs::read_link("/proc/self/ns/pid").ok()?;
        let pid_namespace = pid_namespace.to_str()?;
        let namespace_inode = pid_namespace
            .strip_prefix("pid:[")?
            .strip_suffix(']')?
            .parse::<u64>()
            .ok()?;

        self.start_ticks.store(start_time_ticks, Ordering::Relaxed);
        self.namespace_inode
            .store(namespace_inode, Ordering::Relaxed);
        self.pid.store(pid, Ordering::Release);
        Some((pid, start_time_ticks, pid_namespace.to_string()))
    }
}
