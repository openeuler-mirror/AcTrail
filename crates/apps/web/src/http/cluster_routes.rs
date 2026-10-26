//! Trace API dispatch.

use super::*;

pub(super) fn route_cluster_trace_alerts(
    path: &str,
    query: &str,
    context: &WebContext,
) -> Option<Result<Response, String>> {
    let root = context.cluster_root.as_deref()?;
    let parts = path
        .strip_prefix("/api/traces/")?
        .split('/')
        .collect::<Vec<_>>();
    let [uid, "alerts"] = parts.as_slice() else {
        return None;
    };
    Some(percent_decode(uid).and_then(|uid| {
        view::cluster::with_snapshot(root, &uid, |storage, id| {
            let local_path = format!("/api/traces/{id}/alerts");
            let mut response = alerts::AlertHttp::new(storage, context.alerts)
                .route(&local_path, query)
                .ok_or("trace alerts route unavailable")??;
            if response.status == STATUS_OK {
                let mut body: serde_json::Value =
                    serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
                if let Some(items) = body
                    .get_mut("alerts")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    for item in items {
                        item["trace_id"] = serde_json::Value::String(uid.clone());
                    }
                }
                response.body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
            }
            Ok(response)
        })
    }))
}

pub(super) fn route_cluster_trace_api(
    path: &str,
    query: &str,
    cluster_root: &std::path::Path,
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
        [trace_id, "idle-intervals"] => percent_decode(trace_id)
            .and_then(|uid| {
                view::cluster::with_snapshot(cluster_root, &uid, |storage, id| {
                    let query = idle.parse(id, query)?;
                    view::IdleView::render(storage, &query)
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "counts"] => percent_decode(trace_id)
            .and_then(|uid| {
                view::cluster::with_snapshot(cluster_root, &uid, |storage, id| {
                    view::trace_counts_json(storage, id)
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "metadata"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_metadata_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", "timeline"] => percent_decode(trace_id)
            .and_then(|uid| {
                view::cluster::timeline_page_json(cluster_root, &uid, |id| {
                    limits.parse(&id.to_string(), query)
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),

        [trace_id] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "summary"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_summary_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "time-attribution"] => percent_decode(trace_id)
            .and_then(|trace_id| {
                view::cluster::trace_time_attribution_json(cluster_root, &trace_id)
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "events"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_events_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_payloads_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "timeline"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_timeline_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "processes"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_processes_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "diagnostics"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::trace_diagnostics_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "root"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::action_tree_root_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "children", parent_id] => {
            let trace_id = percent_decode(trace_id);
            let parent_id = percent_decode(parent_id);
            let page = parse_action_tree_page(query);
            match (trace_id, parent_id, page) {
                (Ok(trace_id), Ok(parent_id), Ok(page)) => {
                    view::cluster::action_tree_children_json(
                        cluster_root,
                        &trace_id,
                        &parent_id,
                        page,
                    )
                    .map(Response::json)
                    .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "action-tree", "llm-nav"] => {
            let trace_id = percent_decode(trace_id);
            let mode = parse_llm_nav_mode(query);
            match (trace_id, mode) {
                (Ok(trace_id), Ok(mode)) => view::cluster::action_tree_llm_nav_json(
                    cluster_root,
                    &trace_id,
                    mode.0,
                    mode.1.as_deref(),
                )
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "commands"] => percent_decode(trace_id)
            .and_then(|trace_id| view::cluster::commands_json(cluster_root, &trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id] => {
            let trace_id = percent_decode(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::cluster::action_detail_json(cluster_root, &trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id, "file-path-set"] => {
            let trace_id = percent_decode(trace_id);
            let action_id = percent_decode(action_id);
            let page = parse_action_tree_page(query);
            match (trace_id, action_id, page) {
                (Ok(trace_id), Ok(action_id), Ok(page)) => {
                    view::cluster::file_path_set_json(cluster_root, &trace_id, &action_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "content", "source"] => percent_decode(trace_id)
            .and_then(|trace_id| {
                percent_decode(action_id).and_then(|action_id| {
                    view::cluster::with_snapshot(cluster_root, &trace_id, |storage, local_id| {
                        view::ActionContentView::source(storage, local_id, &action_id)
                    })
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id, "content", "http"] => percent_decode(trace_id)
            .and_then(|trace_id| {
                percent_decode(action_id).and_then(|action_id| {
                    content.http(query).and_then(|page| {
                        view::cluster::with_snapshot(
                            cluster_root,
                            &trace_id,
                            |storage, local_id| {
                                view::ActionContentView::http(storage, local_id, &action_id, &page)
                            },
                        )
                    })
                })
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id, "content", "llm-request"] => {
            let trace_id = percent_decode(trace_id);
            let action_id = percent_decode(action_id);
            let max_bytes = content.prefix(query);
            match (trace_id, action_id, max_bytes) {
                (Ok(trace_id), Ok(action_id), Ok(max_bytes)) => {
                    view::cluster::llm_request_content_json(
                        cluster_root,
                        &trace_id,
                        &action_id,
                        max_bytes,
                    )
                    .map(Response::json)
                    .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "lineage", "llm-request"] => {
            let trace_id = percent_decode(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::cluster::llm_request_lineage_json(cluster_root, &trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }

        [trace_id, "llm-trajectories"] => match percent_decode(trace_id) {
            Ok(trace_id) => view::cluster::llm_trajectory_graph_json(cluster_root, &trace_id)
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        [trace_id, "llm-trajectories", trajectory_id] => {
            let trace_id = percent_decode(trace_id);
            let trajectory_id = percent_decode(trajectory_id);
            match (trace_id, trajectory_id) {
                (Ok(trace_id), Ok(trajectory_id)) => view::cluster::llm_request_trajectory_json(
                    cluster_root,
                    &trace_id,
                    &trajectory_id,
                )
                .map(Response::json)
                .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
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
            let trace_id = percent_decode(trace_id);
            let action_id = percent_decode(action_id);
            let node = content.node(query);
            match (trace_id, action_id, node) {
                (Ok(trace_id), Ok(action_id), Ok(node)) => {
                    view::cluster::llm_request_content_node_json(
                        cluster_root,
                        &trace_id,
                        &action_id,
                        node,
                    )
                    .map(Response::json)
                    .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "payloads", segment_id] => {
            let trace_id = percent_decode(trace_id);
            let segment_id = parse_u64(segment_id);
            match (trace_id, segment_id) {
                (Ok(trace_id), Ok(segment_id)) => {
                    view::cluster::payload_json(cluster_root, &trace_id, segment_id)
                        .map(Response::json)
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        _ => Ok(Response::text(STATUS_NOT_FOUND, "not found")),
    }
}
