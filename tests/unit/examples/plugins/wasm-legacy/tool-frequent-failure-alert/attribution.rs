// Existing attribution tests included in the parent test module.
#[test]
fn llm_queue_is_authoritative_regardless_of_parent_topology() {
    let cfg = test_config(); // 默认 llm_and_mcp + agent_child
    let tool_processes = BTreeSet::new();
    // pending 工具名存在、parent 不是 agent 进程 → 仍判定为 LLM 工具执行
    assert!(matches!(
        classify_command(
            Some("100"),
            "200",
            Some("999"),
            None,
            true,
            &tool_processes,
            &cfg,
        ),
        CommandClassification::ConsumeCandidate
    ));
    // 嵌套在工具进程内的子进程 → 跳过且不消耗工具名
    let mut nested = BTreeSet::new();
    nested.insert("200".to_string());
    assert!(matches!(
        classify_command(Some("100"), "201", Some("200"), None, true, &nested, &cfg,),
        CommandClassification::Skip
    ));
}

#[test]
fn host_backfill_and_agent_self_rules() {
    let cfg = test_config();
    let tool_processes = BTreeSet::new();
    // 宿主回填 command.tool.name → 权威采用
    assert!(matches!(
        classify_command(
            Some("100"),
            "200",
            None,
            Some("bash"),
            false,
            &tool_processes,
            &cfg,
        ),
        CommandClassification::HostTool(name) if name == "bash"
    ));
    // agent 自身进程即使带宿主回填也不算工具执行
    assert!(matches!(
        classify_command(
            Some("100"),
            "100",
            None,
            Some("bash"),
            false,
            &tool_processes,
            &cfg,
        ),
        CommandClassification::Skip
    ));
    // 无 pending、agent_child → 延迟归因候选；any → 按可执行文件名兜底
    assert!(matches!(
        classify_command(Some("100"), "200", None, None, false, &tool_processes, &cfg,),
        CommandClassification::DeferCandidate
    ));
    let mut any_cfg = test_config();
    any_cfg.filter.parent_scope = ParentScope::Any;
    assert!(matches!(
        classify_command(
            Some("100"),
            "200",
            None,
            None,
            false,
            &tool_processes,
            &any_cfg,
        ),
        CommandClassification::FallbackBasename
    ));
    // mcp_only：命令一律不统计
    let mut mcp_cfg = test_config();
    mcp_cfg.filter.tool_scope = ToolScope::McpOnly;
    assert!(matches!(
        classify_command(
            Some("100"),
            "200",
            None,
            Some("bash"),
            true,
            &tool_processes,
            &mcp_cfg,
        ),
        CommandClassification::Skip
    ));
}

#[test]
fn tool_process_marker_prevents_nested_consumption_and_is_released_on_exit() {
    let mut cfg = test_config();
    cfg.filter.llm_attribution = LlmAttribution::Fifo;
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());

    // LLM 返回工具名 bash → 后续命令（parent 非 agent 也归因）
    trace.observe_llm_response(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"name":"bash"}}]"#,
            )],
        ),
        16,
    );
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[("process.id", "200"), ("process.parent.id", "999")],
        ),
        &cfg,
        16,
    );
    assert!(trace.tool_processes.contains("200"));
    assert!(trace.pending_tool_calls.is_empty());

    // 嵌套子进程（parent=200）即使有 pending 名也不消耗
    trace.observe_llm_response(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"function":{"name":"read"}}]"#,
            )],
        ),
        16,
    );
    trace.observe_command_invocation(
        &action_record(
            "command.invocation",
            &[("process.id", "201"), ("process.parent.id", "200")],
        ),
        &cfg,
        16,
    );
    assert_eq!(trace.pending_tool_calls.len(), 1);
    assert_eq!(
        trace
            .pending_tool_calls
            .front()
            .map(|call| call.name.as_str()),
        Some("read")
    );

    // 工具进程退出后释放标记
    trace.observe_process_exit(
        &action_record(
            "process.exit",
            &[("process.id", "200"), ("process.exit_code", "2")],
        ),
        &cfg,
        1001,
    );
    assert!(!trace.tool_processes.contains("200"));
}

#[test]
fn hint_matching_ignores_unrelated_commands() {
    // false 必须是独立 token：不能匹配 core.fsmonitor=false
    assert!(contains_hint("/bin/bash -c false", "false"));
    assert!(!contains_hint("git -c core.fsmonitor=false", "false"));
    assert!(contains_hint(
        "/bin/bash -c nonexistent-command-xyz",
        "nonexistent-command-xyz"
    ));
    assert!(contains_hint("/bin/bash -c ls /lll", "ls /lll"));
    assert!(!contains_hint("git rev-parse --show-toplevel", "ls /lll"));
    assert!(!contains_hint("", "false"));
}

#[test]
fn hint_match_beats_fifo_so_git_noise_does_not_steal_names() {
    let cfg = test_config();
    let mut pending: VecDeque<PendingToolCall> = VecDeque::new();
    pending.push_back(PendingToolCall {
        name: "bash".to_string(),
        hint: "nonexistent-command-xyz".to_string(),
        observed_ms: 1000,
    });
    // git 命令行不包含提示 → 不消耗
    assert_eq!(
        take_pending_tool_call(
            &mut pending,
            "git rev-parse --show-toplevel",
            "/usr/bin/git",
            &cfg,
        ),
        None
    );
    assert_eq!(pending.len(), 1);
    // 真正的 bash 工具命令包含提示 → 消耗
    assert_eq!(
        take_pending_tool_call(
            &mut pending,
            "/bin/bash -c nonexistent-command-xyz",
            "/bin/bash",
            &cfg,
        ),
        Some("bash".to_string())
    );
    assert!(pending.is_empty());
}

#[test]
fn repeated_streaming_tool_call_id_is_enqueued_once() {
    let cfg = test_config();
    let mut trace = TraceState::new(1000);
    let json =
        r#"[{"id":"call-a","function":{"arguments_json":{"command":"ls /a"},"name":"bash"}}]"#;

    let mut first = action_record("llm.response", &[("llm.response.tool_calls_json", json)]);
    first.action_id = "response-a-partial".to_string();
    trace.observe_llm_response_at(&first, &cfg, 1000, 16);

    let mut repeated = first.clone();
    repeated.action_id = "response-a-final".to_string();
    trace.observe_llm_response_at(&repeated, &cfg, 1001, 16);

    assert_eq!(trace.pending_tool_calls.len(), 1);
    assert_eq!(trace.seen_tool_calls.len(), 1);
}

#[test]
fn stale_hinted_call_does_not_block_later_deferred_command() {
    let cfg = test_config();
    let mut trace = TraceState::new(1000);
    trace.agent_process_id = Some("100".to_string());

    trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"id":"call-a","function":{"arguments_json":{"command":"ls /a"},"name":"bash"}}]"#,
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
                ("process.id", "202"),
                ("process.parent.id", "100"),
                ("command.line", "/bin/bash -c ls /b"),
                ("process.executable", "/bin/bash"),
            ],
        ),
        &cfg,
        16,
    );
    assert!(trace.commands_by_process["202"].deferred_attribution);

    let result = trace.observe_process_exit(
        &action_record(
            "process.exit",
            &[("process.id", "202"), ("process.exit_code", "2")],
        ),
        &cfg,
        1001,
    );
    assert!(!result.record);

    let resolved = trace.observe_llm_response_at(
        &action_record(
            "llm.response",
            &[(
                "llm.response.tool_calls_json",
                r#"[{"id":"call-b","function":{"arguments_json":{"command":"ls /b"},"name":"bash"}}]"#,
            )],
        ),
        &cfg,
        1002,
        16,
    );
    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].result.record);
    assert_eq!(resolved[0].result.tool_name, "bash");
    assert_eq!(resolved[0].result.outcome, Outcome::Failure);
}
