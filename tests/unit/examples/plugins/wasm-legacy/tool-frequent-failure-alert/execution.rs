// Existing execution tests included in the parent test module.
#[test]
fn deferred_execution_expires_after_attribution_grace() {
    let mut cfg = test_config();
    cfg.resources.attribution_grace_seconds = 1;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "200"),
                ("process.parent.id", "100"),
                ("command.line", "/bin/bash -c missing-expired"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    trace.observe_process_exit(
        &action_record(
            "process.exit",
            &[("process.id", "200"), ("process.exit_code", "127")],
        ),
        &cfg,
        1000,
    );
    let resolved = trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"arguments_json":{"command":"missing-expired"},"name":"bash"}}]"#,
            )],
        ),
        &cfg,
        2001,
        16,
    );
    assert!(resolved.is_empty());
    assert!(trace.completed_unattributed.is_empty());
}

#[test]
fn heterogeneous_exit_codes_aggregate_under_one_tool() {
    let mut trace = TraceState::new(1000);
    let cfg = test_config();
    // opencode 场景：同一工具三次失败、三个不同退出码（127/2/1）
    let mut alert = None;
    for (index, (action_id, code)) in [("f0", "127"), ("f1", "2"), ("f2", "1")]
        .into_iter()
        .enumerate()
    {
        alert = trace.record_outcome(
            "bash",
            "runtime_error",
            code,
            Outcome::Failure,
            action_id,
            &[action_id],
            1000 + index as u64,
            &cfg,
            "exit code",
            "",
        );
    }
    let alert = alert.expect("3 heterogeneous failures on one tool must alert");
    assert_eq!(alert.failure_count, 3);
    assert_eq!(alert.failure_type, "runtime_error");
    assert_eq!(alert.failure_breakdown.len(), 3);
}

#[test]
fn shell_matrix_parent_scope_any_counts_ls_failures() {
    // 复现回归用例的 shell 矩阵（含真实拓扑）：
    // 根 bash 对 -c 的最后一个简单命令直接 exec 成 ls（同一进程 1），
    // 无 LLM、parent_scope=any、三条 ls 失败（退出码 2）应全部归一到 "ls"。
    let mut cfg = test_config();
    cfg.filter.tool_scope = ToolScope::AgentChildren;
    cfg.filter.parent_scope = ParentScope::Any;
    let mut trace = TraceState::new(1000);
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "1"),
                ("process.parent.id", "0"),
                ("command.line", "bash -c ls /actrail-missing-frequent-a"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    // 三次 ls：pid 2、pid 3 为独立子进程，第三次 bash exec 替换为 ls（仍为 pid 1）
    let commands = [
        ("2", "1", "ls /actrail-missing-frequent-a"),
        ("3", "1", "ls /actrail-missing-frequent-b"),
        ("1", "0", "ls /actrail-missing-frequent-c"),
    ];
    let mut alert = None;
    for (index, (pid, parent, path)) in commands.into_iter().enumerate() {
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", pid),
                    ("process.parent.id", parent),
                    ("command.line", path),
                    ("process.executable", "/usr/bin/ls"),
                ],
            ),
            &cfg,
            16,
        );
        let exit_action_id = format!("exit-{pid}");
        let result = trace.observe_process_exit(
            &action_record(
                "process.exit",
                &[("process.id", pid), ("process.exit_code", "2")],
            ),
            &cfg,
            1000 + index as u64,
        );
        assert!(result.record, "ls failure must be recorded");
        assert_eq!(result.tool_name, "ls");
        assert_eq!(result.outcome, Outcome::Failure);
        let failure_type = cfg.map_failure_type(&result.raw_failure_type);
        let evidence_ids = [result.command_action_id.as_str(), exit_action_id.as_str()];
        alert = trace.record_outcome(
            &result.tool_name,
            &failure_type,
            &result.exit_status,
            result.outcome,
            &exit_action_id,
            &evidence_ids,
            1000 + index as u64,
            &cfg,
            &result.summary,
            &result.command_line,
        );
    }
    let alert = alert.expect("3 ls failures under parent_scope=any must alert");
    assert_eq!(alert.tool_name, "ls");
    assert_eq!(alert.failure_count, 3);
}

#[test]
fn shell_root_exec_replacement_rebinds_failure_to_final_executable() {
    let mut cfg = test_config();
    cfg.filter.tool_scope = ToolScope::AgentChildren;
    cfg.filter.parent_scope = ParentScope::Any;
    let mut trace = TraceState::new(1000);
    // 根 bash（pid 1）先登记，随后 exec 替换为 ls（pid 1）
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "1"),
                ("process.parent.id", "0"),
                ("command.line", "bash -c ls /actrail-missing-frequent-c"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[
                ("process.id", "1"),
                ("process.parent.id", "0"),
                ("command.line", "ls /actrail-missing-frequent-c"),
                ("process.executable", "/usr/bin/ls"),
            ],
        ),
        &cfg,
        16,
    );
    let entry = trace.commands_by_process.get("1").expect("rebound entry");
    assert!(entry.monitored);
    assert_eq!(entry.tool_name, "ls");
    assert!(!entry.llm_attributed);
}

#[test]
fn llm_attributed_exec_replacement_keeps_tool_name() {
    // opencode 场景：bash 工具（LLM 归因）exec 成 ls 后，工具名保持 bash
    let mut cfg = test_config();
    cfg.filter.llm_attribution = LlmAttribution::Fifo;
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
    let entry = trace.commands_by_process.get("296").expect("kept entry");
    assert!(entry.monitored);
    assert_eq!(entry.tool_name, "bash");
    assert!(entry.llm_attributed);
}
