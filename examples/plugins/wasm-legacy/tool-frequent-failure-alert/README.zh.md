# WIT Component 工具频繁失败告警插件

类别：WIT component 观测消费者。

这个示例插件在配置化时间窗口内按 `(trace_id, 工具名)` 聚合工具失败次数与失败率（窗口内
记录失败类型/退出状态的分布，告警中展示主导类别与明细），超过阈值后通过 `alert-write`
把告警写入宿主数据库（SQLite），不做任何外部上报。

它关注**跨工具或跨失败类型的频繁失败**；同一工具是否连续失败由
`tool-consecutive-failure-alert` 独立插件处理。

## 核心特性

- 窗口聚合：时间窗口、失败次数阈值、失败率阈值、触发模式全部可配置；
- 工具维度来自 LLM 返回的工具名（`llm.response.tool_calls_json` 的 `function.name`）或
  MCP 工具名（`mcp.tool.name`），**绝不使用命令行作为聚合键**，避免 opencode 等 agent
  的 bash 命令造成高基数；
- 工具执行以 LLM 工具名队列为权威信号：LLM 返回工具后的下一条非嵌套命令即该
  工具的执行（不要求是 agent 进程的直接子进程，兼容 opencode 等真实进程拓扑），
  工具进程内部的嵌套命令（如 bash 里的 `ls`/`grep`）通过工具进程标记排除；
- 事件源覆盖工具调用/结果（`mcp.tool_call`）、命令执行与进程退出
  （`command.invocation` + `process.exit`）、策略决策（`enforcement.decision`）；
- 无法判定成败时不伪造失败：默认跳过，可选提交 `indeterminate-result` 信息级诊断告警；
- 冷却时间 + 窗口重置 + 宿主去重键（`alert_deduplication_keys`）防止告警风暴与重复落库；
- 默认脱敏 `category_only`：工具参数与结果明文一律不进入告警，只输出工具名、失败摘要
  （可脱敏）、错误类别与证据引用。

## 文件

- `plugin.toml`：插件 manifest。
- `actrail_tool_frequent_failure_alert.wasm`：已编译的 component artifact。
- `frequent-failure-alert-v1.schema.json`：频繁失败告警 payload JSON Schema。
- `indeterminate-result-v1.schema.json`：无法判定诊断告警 payload JSON Schema。
- `src/lib.rs`：Rust 源码。

插件内置默认配置，无需额外配置文件；如需自定义阈值，见下方“插件配置”。

## 工作原理

```
收集器（eBPF / MCP stdio / enforcement）
        ↓
daemon 语义动作投影（command.invocation / process.exit / mcp.tool_call /
                      llm.response(tool_calls_json) / enforcement.decision / agent.identity）
        ↓
插件 consume(batch)
        ├─ 登记 agent 身份（process.id）并解析 LLM 工具名队列
        ├─ mcp.tool_call 直接计成败（mcp.tool.name）
        ├─ command.invocation + process.exit 按 process.id 关联判定成败
        ├─ enforcement.decision 归一为策略失败/成功
        └─ 窗口聚合 (trace, tool) + (failure_type, exit_status) 分布
                ↓ 达到阈值且通过冷却
        alert-write::submit → AlertIngress → SQLite alerts
```

### 工具名解析优先级

1. `command.tool.name`（宿主回填）；
2. 待匹配的 LLM 工具名（`llm.response.tool_calls_json` 的 `function.name`）：
   - 优先用工具参数提示（如 opencode bash 工具的 `arguments.command`）与命令行的
     边界匹配，避免 LLM 响应与工具执行之间的 git 等无关进程抢先消耗工具名；
   - 流式响应最终化晚于工具启动时，先暂存未归因命令及退出结果；在
     `attribution_grace_seconds` 内收到工具调用后反向匹配并回放统计；
   - 同一 tool call 的流式更新按调用 ID 去重，待决提示按归因宽限期回收；
   - 旧提示与当前命令不匹配时，当前命令仍进入延迟缓冲，等待正确响应反向归因；
   - 无参数提示的工具调用按 trace 内 FIFO 兜底；历史未归因命令不做盲目 FIFO；
   - 嵌套在工具进程内的命令不消耗工具名；
   - 已 LLM 归因/宿主回填的进程：exec 替换不覆盖首条登记（工具名保持，
     如 opencode 的 bash 工具 exec 成 ls）；
   - 未归因的进程：按最终可执行文件覆盖登记（bash 对 `-c` 末命令 exec 成
     ls 时，失败记到 ls，与连续失败插件行为一致）；
3. `process.executable` 文件名（仅 `parent_scope=any` 时的原始命令场景）；
4. 无法归因的命令不统计（`parent_scope=agent_child` 时）。

`command.line` 永远不是聚合键；仅当 `debug_include_command_line=true` 时作为脱敏后的附加字段。

### 成败判定

| 事件 | 失败 | 成功 | 无法判定 |
| --- | --- | --- | --- |
| `mcp.tool_call` | `status=error` 或 `mcp.execution.status=error` | `status=success` | 其余状态 |
| `command.invocation` + `process.exit` | `status=error` 或退出码非 0 | `status=success`（`unknown` 可按配置视为成功） | 其余 `unknown` |
| `enforcement.decision` | `result=denied/blocked/error` 或 `status=error` | `result=allowed/success` | 其余 |

## 重新编译

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wasm-legacy/tool-frequent-failure-alert
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/actrail_tool_frequent_failure_alert.wasm .
```

## 插件配置

配置为可选的 JSON（manifest 中 `required = false`），通过 `--plugin-config`
传入；不传时使用内置默认值。完整配置示例：

```json
{
  "alert": {
    "enabled": true,
    "trigger_mode": "count",
    "min_failure_count": 3,
    "min_failure_rate": 0.0,
    "window_seconds": 60,
    "cooldown_seconds": 60,
    "after_alert": "reset_window",
    "indeterminate_handling": "skip",
    "unknown_counts_as_success": true
  },
  "filter": {
    "tool_scope": "llm_and_mcp",
    "parent_scope": "agent_child",
    "llm_attribution": "fifo",
    "monitored_tools": [],
    "ignored_tools": []
  },
  "failure_type_map": {
    "exit_code_nonzero": "runtime_error",
    "command_error": "runtime_error",
    "mcp_error": "mcp_error",
    "policy_denied": "policy_denied",
    "indeterminate": "unknown"
  },
  "strict_mapping": false,
  "evidence": { "max_count": 64 },
  "desensitization": {
    "mode": "category_only",
    "summary_max_chars": 120,
    "redact_keywords": ["sk-", "api_key", "password", "token", "Authorization"]
  },
  "debug_include_command_line": false,
  "reporting": { "mode": "database", "endpoint": "", "enabled": false },
  "resources": {
    "state_ttl_seconds": 600,
    "pending_queue_capacity": 1024,
    "max_trace_states": 1024,
    "attribution_grace_seconds": 10
  }
}
```

主要配置项：

| 配置项 | 默认 | 说明 |
| --- | --- | --- |
| `alert.trigger_mode` | `count` | `count` / `rate` / `count_and_rate` / `count_or_rate` |
| `alert.min_failure_count` | 3 | 窗口内失败次数阈值 |
| `alert.min_failure_rate` | 0.0 | 窗口内失败率阈值 |
| `alert.window_seconds` | 60 | 聚合时间窗口 |
| `alert.cooldown_seconds` | 60 | 同键冷却时间 |
| `alert.after_alert` | `reset_window` | 告警后重置窗口 |
| `alert.indeterminate_handling` | `skip` | 无法判定时跳过或发诊断告警 |
| `alert.unknown_counts_as_success` | true | `process.exit` 状态 `unknown` 且无退出码时按成功计 |
| `filter.tool_scope` | `llm_and_mcp` | `llm_and_mcp` / `mcp_only` / `agent_children` |
| `filter.parent_scope` | `agent_child` | `agent_child`：只统计 LLM/宿主回填的工具执行；`any`：额外允许无工具名的命令按可执行文件名统计（原始命令回归场景） |
| `filter.llm_attribution` | `fifo` | 参数提示优先匹配，无提示时 FIFO 兜底 |
| `filter.monitored_tools` / `ignored_tools` | 空 | 工具过滤（支持尾缀 `*`） |
| `failure_type_map` | 见配置 | 失败信号 → 规范类别 |
| `strict_mapping` | false | 未映射的失败信号是否归入 `other` |
| `evidence.max_count` | 64 | 证据 action id 上限 |
| `desensitization.mode` | `category_only` | `category_only` / `sanitized` / `raw` |
| `reporting.mode` | `database` | 仅落库，`endpoint` 为预留字段 |
| `resources.state_ttl_seconds` | 600 | 无活动 trace 状态回收时间 |
| `resources.pending_queue_capacity` | 1024 | 待决命令/工具名队列上限 |
| `resources.max_trace_states` | 1024 | 同时跟踪的 trace 状态上限 |
| `resources.attribution_grace_seconds` | 10 | 流式 LLM 响应晚到时，未归因命令/退出结果等待反向匹配的秒数；`0` 关闭 |

## 告警 payload

`frequent-failure` 告警的关键字段：

| 字段 | 含义 |
| --- | --- |
| `tool_name` | 工具聚合维度（LLM/MCP 工具名，绝非命令行） |
| `failure_count` / `total_count` / `failure_rate` | 窗口内失败数、调用总数、失败率 |
| `failure_type` / `exit_status` | 窗口内出现最多的失败类别与退出状态 |
| `failure_breakdown` | 窗口内 `(failure_type, exit_status)` 分布明细 |
| `threshold` / `window` | 触发阈值快照与聚合窗口范围（毫秒） |
| `evidence_action_ids` / `first_action_id` / `last_action_id` | 证据 action id 列表与首尾边界 |

完整字段约束见 `frequent-failure-alert-v1.schema.json`。默认脱敏 `category_only`
下不输出 `summary` 与 `debug_command_line`。


## 回测覆盖

仓库回测不是单纯执行几条命令，而是明确分为两层：

| 层次 | 配置 | 启动方式 | 验证重点 |
| --- | --- | --- | --- |
| 直接命令回测 | `tool-frequent-failure-alert.e2e.config.json`，`agent_children + any` | `actrailctl launch -- bash -c '...'` | 成功不告警、三次失败告警、低于阈值、冷却、payload/证据、安装包 |
| 真实 Agent 对话回测 | `tool-frequent-failure-alert.agent.e2e.config.json`，`llm_and_mcp + agent_child` | `actrailctl launch -- opencode run PROMPT` 或 `codex exec PROMPT` | Agent 身份、LLM 工具调用、三次真实工具执行、流式响应晚到归因和告警落库 |

执行完整回测：

```bash
sudo -E python3.11 \
  tests/v2/regression/tool_frequent_failure_alert/run_e2e.py
```

真实 Agent 轮只选择具备工具能力的 `opencode` 或 `codex`。它必须在同一 trace
中同时观察到：

1. Agent 实际执行的三条失败 `ls` 命令；
2. 至少三条来自 `llm.response.tool_calls_json` 的工具调用；
3. 一条满足阈值且带命令/退出证据的 `frequent-failure` 告警。

因此该轮是 Agent 与模型的真实对话和工具调用，不是用普通 `bash`/`ls` 替代
Agent；三次顺序调用还会覆盖重复流式 tool call 去重、旧提示隔离和晚到响应
回放。机器没有可用 Agent 时该子项为 `SKIPPED`；Agent 已执行工具但没有告警
时为 `FAILED`，不会被直接命令轮的成功掩盖。

完整隔离环境命令、SQL 查询和每轮预期结果见回测目录下的 `README.zh.md`。

## 使用

~~~bash
# 启动 daemon
./target/release/actraild start

# 加载插件（告警只写入宿主 SQLite，无外部上报）
./target/release/actraild plugin load \
  --manifest examples/plugins/wasm-legacy/tool-frequent-failure-alert/plugin.toml \
  --instance tool-frequent-alert.test \
  --grant alert-write

# 运行 agent（示例）
./target/release/actrailctl launch --name opencode-test -- opencode run "依次执行三条必然失败的命令并报告结果"
~~~

## 查看告警结果

```bash
sqlite3 /var/lib/actrail/actrail.sqlite \
  "SELECT a.trace_id, d.title, a.payload_json, a.created_at \
   FROM alerts a \
   JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id \
   WHERE d.producer_plugin_id = 'actrail.tool-frequent-failure-alert' \
   ORDER BY a.created_at DESC LIMIT 10;"
```

相关 ABI 说明见 [观测消费者 ABI](../../../../docs/plugins/abi/observation-consumer.zh.md)。
