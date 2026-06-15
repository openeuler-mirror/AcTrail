//! Storage-backed viewer orchestration.

#[path = "storage/export_file.rs"]
mod export_file;
#[path = "storage/export_json.rs"]
mod export_json;
#[path = "storage/export_otel.rs"]
mod export_otel;
#[path = "storage/render.rs"]
mod render;
#[path = "storage/source.rs"]
mod source;

use crate::command::{OutputFormat, StorageCommand, ViewInvocation};
use storage_core::{PayloadRowLimit, PayloadSegmentQuery};

pub fn render_storage_view(invocation: ViewInvocation) -> Result<String, String> {
    if matches!(
        invocation.command,
        StorageCommand::ExportJson | StorageCommand::ExportOtel
    ) {
        match invocation.command {
            StorageCommand::ExportJson => {
                render::reject_limit(invocation.row_limit, "export-json")?;
                return export_json::write_json_export(&invocation);
            }
            StorageCommand::ExportOtel => {
                render::reject_limit(invocation.row_limit, "export-otel")?;
                return export_otel::write_otel_export(&invocation);
            }
            _ => unreachable!("non-export command matched export gate"),
        }
    }

    let storage_config = source::storage_config(&invocation)?;
    let mut storage = source::open_storage(&storage_config)?;
    match invocation.command {
        StorageCommand::Traces => {
            let traces = source::list_traces(storage.as_ref())?;
            Ok(match invocation.output_format {
                OutputFormat::Table => render::render_traces(traces, invocation.row_limit),
                OutputFormat::Json => render::render_traces_json(traces, invocation.row_limit)?,
            })
        }
        StorageCommand::Summary => {
            reject_json_output(invocation.output_format, "summary")?;
            render::reject_limit(invocation.row_limit, "summary")?;
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            Ok(render::render_summary(&snapshot))
        }
        StorageCommand::Processes => {
            reject_json_output(invocation.output_format, "processes")?;
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            Ok(render::render_processes(
                snapshot.memberships,
                invocation.row_limit,
            ))
        }
        StorageCommand::Events => {
            reject_json_output(invocation.output_format, "events")?;
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            Ok(render::render_events(snapshot.events, invocation.row_limit))
        }
        StorageCommand::Network => {
            reject_json_output(invocation.output_format, "network")?;
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            Ok(render::render_network(
                snapshot.events,
                invocation.row_limit,
            ))
        }
        StorageCommand::Payloads => {
            let trace_id = source::resolve_trace_id(storage.as_ref(), invocation.trace_id)?;
            let segments = source::list_payload_segments(
                storage.as_ref(),
                trace_id,
                PayloadSegmentQuery {
                    segment_id: None,
                    direction: invocation.payload_direction,
                    limit: invocation.row_limit.map(payload_row_limit),
                    include_bytes: false,
                },
            )?;
            Ok(match invocation.output_format {
                OutputFormat::Table => render::render_payloads(segments),
                OutputFormat::Json => render::render_payloads_json(segments)?,
            })
        }
        StorageCommand::Payload => {
            reject_json_output(invocation.output_format, "payload")?;
            render::reject_limit(invocation.row_limit, "payload")?;
            let trace_id = source::resolve_trace_id(storage.as_ref(), invocation.trace_id)?;
            let segment_id = invocation
                .payload_segment_id
                .ok_or_else(|| "payload requires --segment-id".to_string())?;
            let format = invocation
                .payload_format
                .ok_or_else(|| "payload requires --format".to_string())?;
            let mut segments = source::list_payload_segments(
                storage.as_ref(),
                trace_id,
                PayloadSegmentQuery {
                    segment_id: Some(segment_id),
                    direction: None,
                    limit: None,
                    include_bytes: true,
                },
            )?;
            let segment = segments
                .pop()
                .ok_or_else(|| format!("payload segment {segment_id} not found"))?;
            Ok(render::render_payload(segment, format))
        }
        StorageCommand::Actions => {
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            let actions = source::list_semantic_actions(storage.as_ref(), snapshot.trace.trace_id)?;
            Ok(match invocation.output_format {
                OutputFormat::Table => {
                    render::render_semantic_actions(actions, invocation.row_limit)
                }
                OutputFormat::Json => {
                    let links = source::list_semantic_action_links(
                        storage.as_ref(),
                        snapshot.trace.trace_id,
                    )?;
                    render::render_semantic_actions_json(actions, links, invocation.row_limit)?
                }
            })
        }
        StorageCommand::Diagnostics => {
            reject_json_output(invocation.output_format, "diagnostics")?;
            let snapshot = source::read_snapshot(storage.as_mut(), invocation.trace_id)?;
            Ok(render::render_diagnostics(
                snapshot.diagnostics,
                invocation.row_limit,
            ))
        }
        StorageCommand::ExportJson | StorageCommand::ExportOtel => {
            unreachable!("export returned before storage render")
        }
    }
}

fn payload_row_limit(limit: crate::command::RowLimit) -> PayloadRowLimit {
    match limit {
        crate::command::RowLimit::Head(count) => PayloadRowLimit::Head(count),
        crate::command::RowLimit::Tail(count) => PayloadRowLimit::Tail(count),
    }
}

fn reject_json_output(format: OutputFormat, command: &'static str) -> Result<(), String> {
    if matches!(format, OutputFormat::Json) {
        return Err(format!("{command} does not support --output-format json"));
    }
    Ok(())
}
