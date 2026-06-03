//! Shared synchronous payload decision execution.

use tls_payload_core::{Decision, PayloadDirection};
use tls_payload_sync::{DecisionEvent, PayloadEvent, SyncEvent};

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
    report_payload(direction, symbol, stream_key, sequence, payload);
    match config.decide(symbol, direction, stream_key, payload) {
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
    send_payload_event(config, direction, symbol, stream_key, sequence, payload);
    if !config.should_print_payload() {
        return;
    }
    output::event_line(&format!(
        "sync_payload: direction={} provider={} symbol={} stream=0x{:x} bytes={} preview={}\n",
        direction.as_str(),
        config.provider(),
        symbol,
        stream_key,
        payload.len(),
        config.redact_payload(payload)
    ));
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
    send_decision_event(
        config, action, direction, symbol, stream_key, sequence, reason,
    );
    if !config.should_print_decision() {
        return;
    }
    output::event_line(&format!(
        "sync_decision: action={action} direction={} provider={} symbol={symbol} bytes={bytes} reason={reason}\n",
        direction.as_str(),
        config.provider(),
    ));
}

fn send_payload_event(
    config: &config::RuntimeConfig,
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    sequence: u64,
    payload: &[u8],
) {
    let Some(trace_id) = config.trace_id() else {
        return;
    };
    let event = SyncEvent::Payload(PayloadEvent {
        trace_id,
        pid: process_id(),
        direction,
        provider: config.provider().to_string(),
        symbol: symbol.to_string(),
        stream_key: stream_key as u64,
        sequence,
        bytes: payload.to_vec(),
    });
    send_event_or_abort(config, &event);
}

fn send_decision_event(
    config: &config::RuntimeConfig,
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
    let event = SyncEvent::Decision(DecisionEvent {
        trace_id,
        pid: process_id(),
        direction,
        provider: config.provider().to_string(),
        symbol: symbol.to_string(),
        stream_key: stream_key as u64,
        sequence,
        action: action.to_string(),
        reason: reason.to_string(),
    });
    send_event_or_abort(config, &event);
}

fn send_event_or_abort(config: &config::RuntimeConfig, event: &SyncEvent) {
    if let Err(error) = config.send_event(event) {
        output::error_line(&format!("tls_payload_probe_sync event error: {error}\n"));
        unsafe {
            libc::_exit(126);
        }
    }
}

fn process_id() -> u32 {
    unsafe { libc::getpid() as u32 }
}
