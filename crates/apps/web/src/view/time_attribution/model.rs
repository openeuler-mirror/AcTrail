use super::*;

#[derive(Clone, Debug)]
pub(super) struct ToolCallOccurrence {
    pub(super) action_id: String,
    pub(super) name: String,
    pub(super) observed_at: u128,
}

impl ToolCallOccurrence {
    pub(super) fn observed_in(&self, window: Interval) -> bool {
        self.observed_at >= window.start && self.observed_at < window.end
    }
}

pub(super) fn tool_call_occurrences(
    actions: &[SemanticAction],
    observation_window: Option<Interval>,
) -> Vec<ToolCallOccurrence> {
    let actions_by_id = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect::<HashMap<_, _>>();
    let accepted_response_ids = actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
        .filter_map(|action| {
            let request_id = action
                .attributes
                .get(attr_keys::llm_call::REQUEST_ACTION_ID)?;
            let request = actions_by_id.get(request_id.as_str())?;
            (request.kind == SemanticActionKind::LlmRequest
                && !request
                    .attributes
                    .contains_key(attr_keys::llm_request::BACKGROUND_KIND))
            .then_some(action)
        })
        .filter_map(|action| {
            action
                .attributes
                .get(attr_keys::llm_call::RESPONSE_ACTION_ID)
        })
        .map(String::as_str)
        .collect::<HashSet<_>>();
    actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmToolCall)
        .filter_map(|action| {
            let name = non_empty_attr(action, attr_keys::llm_tool_call::NAME)?;
            let response_id = action
                .attributes
                .get(attr_keys::llm_tool_call::RESPONSE_ACTION_ID)?;
            if !accepted_response_ids.contains(response_id.as_str()) {
                return None;
            }
            let observed_at = system_time_nanos(action.start_time).ok()?;
            if observation_window
                .is_some_and(|window| observed_at < window.start || observed_at >= window.end)
            {
                return None;
            }
            Some(ToolCallOccurrence {
                action_id: action.action_id.clone(),
                name,
                observed_at,
            })
        })
        .collect()
}

pub(super) fn model_intervals(
    actions: &[SemanticAction],
    scope: Interval,
    provisional: bool,
    tracker: &mut StatusTracker,
) -> Vec<ModelInterval> {
    let actions_by_id = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let mut intervals = Vec::new();
    for action in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
    {
        let request = action
            .attributes
            .get(attr_keys::llm_call::REQUEST_ACTION_ID)
            .and_then(|request_id| actions_by_id.get(request_id.as_str()).copied())
            .filter(|request| request.kind == SemanticActionKind::LlmRequest);
        let Some(request) = request else {
            tracker.action_warning(
                "llm_call_request_missing",
                "LLM call has no matching request action and is left unattributed.",
                &action.action_id,
                None,
            );
            continue;
        };
        if let Some(background_kind) = request
            .attributes
            .get(attr_keys::llm_request::BACKGROUND_KIND)
        {
            tracker.action_info(
                "background_llm_call_excluded",
                format!(
                    "LLM call is classified as background activity ({background_kind}) and is excluded from user-turn attribution."
                ),
                &action.action_id,
                None,
            );
            continue;
        }
        let response = action
            .attributes
            .get(attr_keys::llm_call::RESPONSE_ACTION_ID)
            .and_then(|response_id| actions_by_id.get(response_id.as_str()).copied())
            .filter(|response| response.kind == SemanticActionKind::LlmResponse);
        let model = resolved_call_model(action, request, response, tracker);
        let Some(response) = response else {
            report_unpaired_call(action, provisional, tracker);
            continue;
        };
        let Ok(start) = system_time_nanos(request.start_time) else {
            tracker.action_error(
                "llm_call_clock_invalid",
                "LLM request start time cannot be represented.",
                &action.action_id,
            );
            continue;
        };
        let end = match response.end_time {
            Some(end) => match system_time_nanos(end) {
                Ok(end) => end,
                Err(_) => {
                    tracker.action_error(
                        "llm_call_clock_invalid",
                        "LLM response end time cannot be represented.",
                        &action.action_id,
                    );
                    continue;
                }
            },
            None => {
                if provisional {
                    tracker.action_info(
                        "llm_response_pending",
                        "LLM response has no observable data boundary yet; pending time remains unattributed.",
                        &action.action_id,
                        None,
                    );
                } else {
                    tracker.action_warning(
                        "llm_response_end_missing",
                        "Terminal Trace contains an LLM response without an observable end; the call is left unattributed.",
                        &action.action_id,
                        None,
                    );
                }
                continue;
            }
        };
        if end < start {
            tracker.action_error(
                "llm_call_clock_reversed",
                "LLM call end precedes its start and is left unattributed.",
                &action.action_id,
            );
            continue;
        }
        let Some(turn_key) = user_turn_key(request) else {
            tracker.action_info(
                "llm_call_without_user_message",
                "LLM call has no retained user-message evidence and is excluded from user-turn attribution.",
                &action.action_id,
                Interval::new(start, end),
            );
            continue;
        };
        let user_input_start = request
            .attributes
            .get(attr_keys::agent_turn::USER_INPUT_OBSERVED_AT_UNIX_NANOS)
            .and_then(|value| value.parse::<u128>().ok())
            .filter(|input_start| *input_start <= start && *input_start >= scope.start);
        let finalized_on_trace_close =
            finalized_on_trace_close(action) || finalized_on_trace_close(response);
        if finalized_on_trace_close && end == scope.end {
            tracker.action_warning(
                "llm_call_trace_close_only",
                "LLM call has no response boundary distinct from Trace close and is left unattributed.",
                &action.action_id,
                Interval::new(start.min(scope.end), scope.end),
            );
            continue;
        }
        let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
        else {
            continue;
        };
        let partial_observation = action.completeness != SemanticActionCompleteness::Complete
            || response.completeness != SemanticActionCompleteness::Complete
            || response.status == SemanticActionStatus::InProgress;
        if finalized_on_trace_close {
            tracker.action_warning(
                "llm_call_closed_on_trace_end",
                "LLM call was finalized at Trace close; only its last observed response boundary is attributed.",
                &action.action_id,
                Some(interval),
            );
        } else if partial_observation {
            if provisional {
                tracker.action_info(
                    "llm_call_in_progress",
                    "LLM response is still open; attribution stops at the latest observed response data.",
                    &action.action_id,
                    Some(interval),
                );
            } else {
                tracker.action_warning(
                    "llm_call_partial",
                    "LLM call is incomplete; attribution stops at the last reliable response boundary.",
                    &action.action_id,
                    Some(interval),
                );
            }
        }
        intervals.push(ModelInterval {
            interval,
            action_id: action.action_id.clone(),
            model,
            process: request.process.clone(),
            status: if provisional && response.status == SemanticActionStatus::InProgress {
                "in_progress"
            } else if finalized_on_trace_close || partial_observation {
                "partial"
            } else if response.status == SemanticActionStatus::Error {
                "error"
            } else {
                "complete"
            },
            turn_key,
            user_input_start,
        });
    }
    intervals.sort_by(|left, right| {
        (left.interval.start, left.interval.end, &left.action_id).cmp(&(
            right.interval.start,
            right.interval.end,
            &right.action_id,
        ))
    });
    intervals
}

fn user_turn_key(request: &SemanticAction) -> Option<UserTurnKey> {
    let user_message_count = request
        .attributes
        .get(attr_keys::llm_request::USER_MESSAGE_COUNT)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|count| *count > 0);
    let latest_user_message_hash = request
        .attributes
        .get(attr_keys::llm_request::LATEST_USER_MESSAGE_HASH)
        .filter(|value| valid_user_message_hash(value))
        .cloned();
    let (user_message_count, latest_user_message_hash) =
        match (user_message_count, latest_user_message_hash) {
            (Some(count), Some(hash)) => (count, hash),
            _ => {
                let preview = request
                    .attributes
                    .get(attr_keys::llm_request::MESSAGE_PREVIEW)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())?;
                (0, format!("legacy-preview:{preview}"))
            }
        };
    Some(UserTurnKey {
        process: request.process.clone(),
        user_message_count,
        latest_user_message_hash,
    })
}

fn valid_user_message_hash(value: &str) -> bool {
    value.len() == "sha256:".len() + 64
        && value.starts_with("sha256:")
        && value["sha256:".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn report_unpaired_call(action: &SemanticAction, provisional: bool, tracker: &mut StatusTracker) {
    if provisional && action.status == SemanticActionStatus::InProgress {
        tracker.action_info(
            "llm_call_unpaired",
            "LLM request has no matching response yet; it is excluded from model-side time until response evidence arrives.",
            &action.action_id,
            None,
        );
    } else {
        tracker.action_warning(
            "llm_call_unpaired",
            "LLM call has no matching response and is excluded from model-side time.",
            &action.action_id,
            None,
        );
    }
}

fn resolved_call_model(
    call: &SemanticAction,
    request: &SemanticAction,
    response: Option<&SemanticAction>,
    tracker: &mut StatusTracker,
) -> Option<String> {
    let request_model = validated_action_model(request, attr_keys::llm_request::MODEL);
    let response_model =
        response.and_then(|action| validated_action_model(action, attr_keys::llm_response::MODEL));
    let call_model = validated_action_model(call, attr_keys::llm_call::MODEL);
    let has_invalid_model = [
        action_model_is_invalid(request, attr_keys::llm_request::MODEL),
        response
            .is_some_and(|action| action_model_is_invalid(action, attr_keys::llm_response::MODEL)),
        action_model_is_invalid(call, attr_keys::llm_call::MODEL),
    ]
    .into_iter()
    .any(|invalid| invalid);
    if has_invalid_model {
        tracker.action_info(
            "llm_model_invalid",
            "A malformed model identifier was ignored; JSON fragments are not used as model keys.",
            &call.action_id,
            None,
        );
    }
    if let (Some(request_model), Some(response_model)) = (&request_model, &response_model)
        && !model_identifiers_equal(request_model, response_model)
    {
        tracker.action_info(
            "llm_model_conflict",
            "Request and response report different model identifiers; the response model is used.",
            &call.action_id,
            None,
        );
    }
    response_model.or(request_model).or(call_model)
}

fn model_identifiers_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right.trim())
}

fn validated_action_model(action: &SemanticAction, key: &str) -> Option<String> {
    action
        .attributes
        .get(key)
        .and_then(|value| validated_model_identifier(value))
        .map(ToOwned::to_owned)
}

fn action_model_is_invalid(action: &SemanticAction, key: &str) -> bool {
    action
        .attributes
        .get(key)
        .is_some_and(|value| validated_model_identifier(value).is_none())
}

fn finalized_on_trace_close(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(attr_keys::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
        .is_some_and(|value| value == "true")
}
