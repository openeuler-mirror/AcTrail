// Existing replay tests included in the parent test module.
#[test]
fn sequential_agent_calls_with_late_responses_reach_threshold() {
    let mut cfg = test_config();
    cfg.alert.min_failure_count = 3;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());
    let mut final_alert = None;

    for (index, path) in ["/a", "/b", "/c"].into_iter().enumerate() {
        let pid = format!("20{index}");
        let command_line = format!("/bin/bash -c ls {path}");
        let command_hint = format!("ls {path}");
        let call_json = format!(
            r#"[{{"id":"call-{index}","function":{{"arguments_json":{{"command":"{command_hint}"}},"name":"bash"}}}}]"#
        );

        if index == 0 {
            trace.observe_llm_response_at(
                &action_record(
                    "llm.response",
                    &[("llm.response.tool_calls_json", &call_json)],
                ),
                &cfg,
                1000,
                16,
            );
        }

        let mut command = action_record(
            "command.invocation",
            &[
                ("process.id", &pid),
                ("process.parent.id", "100"),
                ("command.line", &command_line),
                ("process.executable", "/bin/bash"),
            ],
        );
        command.action_id = format!("command-{index}");
        trace.observe_command_invocation(&command, &cfg, 16);

        let mut exit = action_record(
            "process.exit",
            &[("process.id", &pid), ("process.exit_code", "2")],
        );
        exit.action_id = format!("exit-{index}");
        let immediate = trace.observe_process_exit(&exit, &cfg, 1001 + index as u64);

        let executions = if immediate.record {
            vec![DeferredExecution {
                result: immediate,
                exit_action_id: exit.action_id.clone(),
                observed_ms: 1001 + index as u64,
            }]
        } else {
            let mut response = action_record(
                "llm.response",
                &[("llm.response.tool_calls_json", &call_json)],
            );
            response.action_id = format!("response-{index}");
            trace.observe_llm_response_at(&response, &cfg, 1002 + index as u64, 16)
        };

        for execution in executions {
            let result = execution.result;
            final_alert = trace.record_outcome(
                &result.tool_name,
                &cfg.map_failure_type(&result.raw_failure_type),
                &result.exit_status,
                result.outcome,
                &execution.exit_action_id,
                &[&result.command_action_id, &execution.exit_action_id],
                1002 + index as u64,
                &cfg,
                &result.summary,
                &result.command_line,
            );
        }

        if index == 0 {
            let mut repeated = action_record(
                "llm.response",
                &[("llm.response.tool_calls_json", &call_json)],
            );
            repeated.action_id = "response-0-repeated".to_string();
            trace.observe_llm_response_at(&repeated, &cfg, 1003, 16);
        }
    }

    let alert = final_alert.expect("three sequential failures must alert");
    assert_eq!(alert.tool_name, "bash");
    assert_eq!(alert.failure_count, 3);
}

#[test]
fn exec_replacement_does_not_override_monitored_command() {
    let cfg = test_config();
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());
    trace.observe_llm_response(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"arguments_json":{"command":"ls /lll"},"name":"bash"}}]"#,
            )],
        ),
        16,
    );
    // 第一次：bash 工具命令，命中提示
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "296"),
                ("process.parent.id", "100"),
                ("command.line", "/bin/bash -c ls /lll"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    // 第二次：同一进程 exec 替换为 ls，不能覆盖首条登记
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "296"),
                ("process.parent.id", "100"),
                ("command.line", "ls /lll"),
                ("process.executable", "/usr/bin/ls"),
            ],
        ),
        &cfg,
        16,
    );
    let entry = trace
        .commands_by_process
        .get("296")
        .expect("first registration kept");
    assert!(entry.monitored);
    assert_eq!(entry.tool_name, "bash");
    assert!(trace.pending_tool_calls.is_empty());
}

#[test]
fn terminal_trace_state_survives_attribution_grace() {
    let mut trace = TraceState::new(1000);
    trace.terminal_ms = Some(1000);
    let grace_ms = 10_000;
    assert!(2000_u64.saturating_sub(trace.terminal_ms.unwrap()) < grace_ms);
    assert!(!(11_001_u64.saturating_sub(trace.terminal_ms.unwrap()) < grace_ms));
}

#[test]
fn command_before_llm_response_is_attributed_before_exit() {
    let mut cfg = test_config();
    cfg.alert.min_failure_count = 1;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());

    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "200"),
                ("process.parent.id", "100"),
                ("command.line", "/bin/bash -c ls /late-running"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    assert!(trace.commands_by_process["200"].deferred_attribution);

    let resolved = trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"arguments_json":{"command":"ls /late-running"},"name":"bash"}}]"#,
            )],
        ),
        &cfg,
        1001,
        16,
    );
    assert!(resolved.is_empty());
    let entry = &trace.commands_by_process["200"];
    assert!(entry.llm_attributed);
    assert!(!entry.deferred_attribution);
    assert_eq!(entry.tool_name, "bash");

    let result = trace.observe_process_exit(
        &action_record(
            "process.exit",
            &[("process.id", "200"), ("process.exit_code", "2")],
        ),
        &cfg,
        1002,
    );
    assert!(result.record);
    assert_eq!(result.tool_name, "bash");
    assert_eq!(result.outcome, Outcome::Failure);
}

#[test]
fn in_progress_exit_waits_only_without_terminal_evidence() {
    let mut cfg = test_config();
    cfg.alert.min_failure_count = 1;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());
    trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"arguments_json":{"command":"ls /progress"},"name":"bash"}}]"#,
            )],
        ),
        &cfg,
        1000,
        16,
    );
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "200"),
                ("process.parent.id", "100"),
                ("command.line", "/bin/bash -c ls /progress"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );

    let mut pending_exit = action_record("process.exit", &[("process.id", "200")]);
    pending_exit.status = "in_progress".to_string();
    let pending = trace.observe_process_exit(&pending_exit, &cfg, 1001);
    assert!(pending.matched);
    assert!(!pending.record);
    assert!(trace.commands_by_process.contains_key("200"));

    let mut final_exit = action_record(
        "process.exit",
        &[("process.id", "200"), ("process.exit_code", "2")],
    );
    final_exit.status = "in_progress".to_string();
    let result = trace.observe_process_exit(&final_exit, &cfg, 1002);
    assert!(result.matched);
    assert!(result.record);
    assert_eq!(result.tool_name, "bash");
    assert_eq!(result.outcome, Outcome::Failure);
    assert!(!trace.commands_by_process.contains_key("200"));
}

#[test]
fn completed_commands_are_replayed_after_streaming_llm_response() {
    let mut cfg = test_config();
    cfg.alert.min_failure_count = 2;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());

    // OpenCode 内部 git 命令也是 agent 子进程，但不能抢走带参数提示的工具调用。
    for (pid, line, code, command_id, exit_id) in [
        (
            "200",
            "git rev-parse --show-toplevel",
            "0",
            "git-command",
            "git-exit",
        ),
        (
            "201",
            "/bin/bash -c missing-late-a",
            "127",
            "command-a",
            "exit-a",
        ),
        (
            "202",
            "/bin/bash -c missing-late-b",
            "127",
            "command-b",
            "exit-b",
        ),
    ] {
        let mut command = action_record(
            "command.invocation",
            &[
                ("process.id", pid),
                ("process.parent.id", "100"),
                ("command.line", line),
                ("process.executable", "/bin/bash"),
            ],
        );
        command.action_id = command_id.to_string();
        trace.observe_command_invocation(&command, &cfg, 16);
        let mut exit = action_record(
            "process.exit",
            &[("process.id", pid), ("process.exit_code", code)],
        );
        exit.action_id = exit_id.to_string();
        let result = trace.observe_process_exit(&exit, &cfg, 1001);
        assert!(!result.record, "unattributed result must be deferred");
    }
    assert_eq!(trace.completed_unattributed.len(), 3);

    let resolved = trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[
                    {"function":{"arguments_json":{"command":"missing-late-a"},"name":"bash"}},
                    {"function":{"arguments_json":{"command":"missing-late-b"},"name":"bash"}}
                ]"#,
            )],
        ),
        &cfg,
        1002,
        16,
    );
    assert_eq!(resolved.len(), 2);
    assert_eq!(
        trace.completed_unattributed.len(),
        1,
        "git remains unmatched"
    );

    let mut alert = None;
    for execution in resolved {
        let result = execution.result;
        let failure_type = cfg.map_failure_type(&result.raw_failure_type);
        alert = trace.record_outcome(
            &result.tool_name,
            &failure_type,
            &result.exit_status,
            result.outcome,
            &execution.exit_action_id,
            &[&result.command_action_id, &execution.exit_action_id],
            1002,
            &cfg,
            &result.summary,
            &result.command_line,
        );
    }
    let alert = alert.expect("two replayed failures must alert");
    assert_eq!(alert.tool_name, "bash");
    assert_eq!(alert.failure_count, 2);
    assert_eq!(alert.total_count_for_test(), 2);
}
