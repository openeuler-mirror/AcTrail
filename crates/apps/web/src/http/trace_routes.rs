//! Trace API dispatch.

use super::*;

pub(super) fn route_trace_api(
    path: &str,
    query: &str,
    storage: &StorageConfig,
    limits: &timeline::TimelineLimits,
    idle: &idle::IdleLimits,
    content: &query::ContentLimits,
) -> Result<Response, String> {
    let parts = path
        .strip_prefix("/api/traces/")
        .map(|suffix| suffix.split('/').collect::<Vec<_>>());
    let Some(parts) = parts else {
        return Ok(Response::text(STATUS_NOT_FOUND, "not found"));
    };
    match parts.as_slice() {
        [trace_id, "idle-intervals"] => {
            match parse_u64(trace_id).and_then(|id| idle.parse(id, query)) {
                Ok(query) => view::IdleView::render(storage, &query).map(Response::json),
                Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "counts"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_counts_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "metadata"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_metadata_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", "timeline"] => match limits.parse(trace_id, query) {
            Ok(query) => view::TimelineView::render(storage, &query).map(Response::json),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },

        [trace_id] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "summary"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_summary_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "time-attribution"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_time_attribution_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "events"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_events_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_payloads_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "timeline"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_timeline_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "processes"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_processes_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "diagnostics"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_diagnostics_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "root"] => parse_u64(trace_id)
            .and_then(|trace_id| view::action_tree_root_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "children", parent_id] => {
            let trace_id = parse_u64(trace_id);
            let parent_id = percent_decode(parent_id);
            let page = parse_action_tree_page(query);
            match (trace_id, parent_id, page) {
                (Ok(trace_id), Ok(parent_id), Ok(page)) => {
                    view::action_tree_children_json(storage, trace_id, &parent_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "action-tree", "llm-nav"] => {
            let trace_id = parse_u64(trace_id);
            let mode = parse_llm_nav_mode(query);
            match (trace_id, mode) {
                (Ok(trace_id), Ok(mode)) => {
                    view::action_tree_llm_nav_json(storage, trace_id, mode.0, mode.1.as_deref())
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::action_detail_json(storage, trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id, "file-path-set"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let page = parse_action_tree_page(query);
            match (trace_id, action_id, page) {
                (Ok(trace_id), Ok(action_id), Ok(page)) => {
                    view::action_file_path_set_json(storage, trace_id, &action_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "content", "source"] => parse_u64(trace_id)
            .and_then(|trace_id| {
                percent_decode(action_id).and_then(|action_id| {
                    view::ActionContentView::source(storage, trace_id, &action_id)
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id, "content", "http"] => parse_u64(trace_id)
            .and_then(|trace_id| {
                percent_decode(action_id).and_then(|action_id| {
                    content.http(query).and_then(|page| {
                        view::ActionContentView::http(storage, trace_id, &action_id, &page)
                    })
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id, "content", "llm-request"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let max_bytes = content.prefix(query);
            match (trace_id, action_id, max_bytes) {
                (Ok(trace_id), Ok(action_id), Ok(max_bytes)) => {
                    view::action_llm_request_content_json(storage, trace_id, &action_id, max_bytes)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "lineage", "llm-request"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::action_llm_request_lineage_json(storage, trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }

        [trace_id, "llm-trajectories"] => match parse_u64(trace_id) {
            Ok(trace_id) => view::llm_trajectory_graph_json(storage, trace_id)
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        [trace_id, "llm-trajectories", trajectory_id] => {
            let trace_id = parse_u64(trace_id);
            let trajectory_id = percent_decode(trajectory_id);
            match (trace_id, trajectory_id) {
                (Ok(trace_id), Ok(trajectory_id)) => {
                    view::llm_request_trajectory_json(storage, trace_id, &trajectory_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id, "content", "mcp-jsonrpc"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let max_bytes = content.prefix(query);
            match (trace_id, action_id, max_bytes) {
                (Ok(trace_id), Ok(action_id), Ok(max_bytes)) => {
                    view::action_mcp_jsonrpc_content_json(storage, trace_id, &action_id, max_bytes)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [
            trace_id,
            "actions",
            action_id,
            "content",
            "llm-request",
            "node",
        ] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let node = content.node(query);
            match (trace_id, action_id, node) {
                (Ok(trace_id), Ok(action_id), Ok(node)) => {
                    view::action_llm_request_content_node_json(storage, trace_id, &action_id, node)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "commands"] => parse_u64(trace_id)
            .and_then(|trace_id| view::commands_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads", segment_id] => {
            let trace_id = parse_u64(trace_id);
            let segment_id = parse_u64(segment_id);
            match (trace_id, segment_id) {
                (Ok(trace_id), Ok(segment_id)) => {
                    view::payload_json(storage, trace_id, segment_id).map(Response::json)
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        _ => Ok(Response::text(STATUS_NOT_FOUND, "not found")),
    }
}
