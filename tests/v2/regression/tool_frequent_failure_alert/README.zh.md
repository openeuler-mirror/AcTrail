# Tool Frequent Failure Alert 回归

## 测试目标

该用例验证 `wasm-legacy/tool-frequent-failure-alert` 插件的端到端告警链路：

- 插件通过 `alert-write` 授权加载，消费 `agent.identity`、`llm.response`、
  `command.invocation`、`process.exit`、`mcp.tool_call` 与
  `enforcement.decision` 语义动作，在配置化时间窗口内按
  `(trace_id, 工具名)` 聚合失败（窗口内记录失败类型/退出状态分布）；
- 同一 trace 内同一工具在窗口内失败达到阈值（默认 3）后，`frequent-failure`
  告警定义被注册且告警落库；
- 同一工具的成功命令（退出码 0）不计为失败，并计入 `total_count` 和失败率
  分母；混合正向轮以三次成功、三次失败断言 `3/6=0.5`；
- 失败次数低于窗口阈值时不告警；
- 超过阈值的持续失败被冷却抑制，同一 trace 只落库一条告警；
- 告警 payload 包含 trace id、工具聚合维度、失败计数、窗口范围与证据
  action id 列表，且默认不包含工具参数明文；
- `scripts/install-release.sh` 安装出的官方插件包可被发现并从安装目录加载；
- 机器上存在真实 Agent 时，用统一提示词驱动 Agent 执行三条失败命令并验证
  告警；没有可用 Agent 时该轮次跳过，不计失败。

## 判定语义

- AcTrail 自身产物缺失或不可执行（`actraild`、`actrailctl`、
  `actrailviewer`、release 安装的插件包）判定为 `FAILED`，不降级为
  `SKIPPED`；
- 真实 Agent 缺失或不可用（`opencode`、`codex` 均未找到）时，真实 Agent
  子项判定为 `SKIPPED`，不影响整体 `PASSED`；
- 真实 Agent 执行了三条失败命令但无告警落库，或者告警存在但同一 trace
  没有采集到至少三条 LLM 工具调用时，判定为 `FAILED`。

## 回测组成

| 回测层次 | 运行配置 | 执行主体 | 自动断言 |
| --- | --- | --- | --- |
| 直接命令回测 | `tool-frequent-failure-alert.e2e.config.json`（`agent_children + any`） | `actrailctl launch -- bash -c ...` | 同一 `ls` 工具三次成功加三次失败后告警并得到 `total_count=6`、`failure_rate=0.5`，低于阈值不告警、冷却只告警一次、payload/证据与安装包 |
| 真实 Agent 对话回测 | `tool-frequent-failure-alert.agent.e2e.config.json`（`llm_and_mcp + agent_child`） | OpenCode `run` 或 Codex `exec` 与模型真实对话后调用工具 | 三次失败 `ls`、至少三条 `llm.response.tool_calls_json` 工具调用、告警及命令/退出证据 |

第二层不是把 Agent 名称包在普通 shell 外面：自动回测直接启动 Agent CLI，
由模型根据提示词决定并发起三次工具调用，再从该 trace 的采集动作证明这些
调用确实来自 LLM 响应。

# Quick Run

在仓库根目录以 root 执行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case tool_frequent_failure_alert
```

公共 runner 会先执行 `scripts/install-release.sh`（构建并安装官方插件包），再
创建隔离工作目录，启动真实 `actraild`，加载插件后依次跑混合正向、低于
阈值、冷却、安装包和可选真实 Agent 轮次。冷缓存机器可能长时间停留在
release、TLS runtime 或 WASM 插件编译阶段；只要 `cargo`/`rustc` 仍在占用
CPU，就不是测试卡死。

只运行该 case 并显示详细过程：

```bash
sudo -E python3.11 \
  tests/v2/regression/tool_frequent_failure_alert/run_e2e.py
```

# 步骤摘要

1. 检查 release 二进制（`actraild`、`actrailctl`、`actrailviewer`）与
   root/eBPF 测试权限。
2. 断言 `scripts/install-release.sh` 已把插件安装到
   `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/tool-frequent-failure-alert/`
   （manifest、wasm、两个 payload schema）。
3. 在隔离目录初始化、清理并启动 AcTrail。
4. 从仓库示例目录加载插件，携带测试专用配置
   `tool-frequent-failure-alert.e2e.config.json`（`tool_scope=agent_children`、
   `parent_scope=any`），并授予 `alert-write`。
5. 混合正向轮：同一 `ls` 工具先成功三次、再失败三次，断言定义注册、恰好
   一条告警，并检查 `failure_count=3`、`total_count=6`、
   `failure_rate=0.5` 以及 payload 字段与
   证据（`evidence_action_ids` 唯一且同时含 `:command.invocation` 与
   `:process.exit` 动作 id）正确。
6. 低于阈值轮：同一 trace 只有两条失败 `ls`，断言无告警。
7. 冷却矩阵：单 trace 连续五条失败命令，断言仍只有一条告警，且存储的
   `command.invocation` 数量证明额外失败确实发生（冷却抑制重复告警）。
8. 断言仓库实例 `observed_records > 0`。
9. 安装包轮：临时卸载仓库实例，仅加载安装目录实例跑一轮正向（同一时刻
    只保留一个实例，避免两个实例对同一批事件各提交一条重复告警），断言
    安装产物可用；完成后恢复仓库实例。
10. 真实 Agent 轮（可选）：切换到
    `tool-frequent-failure-alert.agent.e2e.config.json`
    （`tool_scope=llm_and_mcp`、`parent_scope=agent_child`），再探测
    `opencode`/`codex`，用统一提示词驱动其执行三条失败命令，覆盖工具命令/
    退出早于流式 `llm.response` 最终化的延迟归因，以及重复 tool call 去重和
    旧提示不阻塞后续顺序调用；最多执行两次，以容忍真实 Agent/采集首轮未产生
    完整退出证据。告警 payload 必须满足 `failure_count=3`、
    `total_count=3`、`failure_rate=1.0`，并至少包含三组命令/退出证据。
    无可用 Agent 或 Agent 未实际执行失败命令时 `SKIPPED`；
    Agent 执行了失败命令但无告警，或同一 trace 未采集到至少三条 LLM 工具调用
    时，判定为 `FAILED`。
11. 卸载插件、停止 daemon、清理。

# 手动测试

以下命令完整复现自动测试，均从仓库根目录、在同一个 root shell 中执行。
手动测试使用隔离的 `/tmp/actrail-frequent-alert-manual` 工作目录，不读取或
清理系统默认的 `/var/lib/actrail` 数据。

## 步骤1：检查测试前提并构建 release

### 手动指令

```bash
test "$(id -u)" -eq 0
command -v python3 >/dev/null
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
test -f examples/plugins/wasm-legacy/tool-frequent-failure-alert/plugin.toml
test -f examples/plugins/wasm-legacy/tool-frequent-failure-alert/actrail_tool_frequent_failure_alert.wasm

bash scripts/install-release.sh
```

### 预期结果

当前用户为 root；三个测试所需二进制均可执行；插件 manifest 与 wasm 产物
存在；安装脚本成功并把插件安装到
`${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}`。

## 步骤2：确认安装包并创建隔离配置启动 daemon

### 手动指令

```bash
PLUGIN_ROOT="${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}"
test -f "$PLUGIN_ROOT/tool-frequent-failure-alert/tool-frequent-failure-alert.plugin.toml"
test -f "$PLUGIN_ROOT/tool-frequent-failure-alert/actrail_tool_frequent_failure_alert.wasm"
test -f "$PLUGIN_ROOT/tool-frequent-failure-alert/frequent-failure-alert-v1.schema.json"
test -f "$PLUGIN_ROOT/tool-frequent-failure-alert/indeterminate-result-v1.schema.json"

REPO="$(pwd -P)"
BIN="$REPO/target/release"
WORK=/tmp/actrail-frequent-alert-manual
E2E_CONFIG="$REPO/tests/v2/regression/tool_frequent_failure_alert/tool-frequent-failure-alert.e2e.config.json"

AGENT_CONFIG="$REPO/tests/v2/regression/tool_frequent_failure_alert/tool-frequent-failure-alert.agent.e2e.config.json"
mkdir -p "$WORK/run" "$WORK/log" "$WORK/data" "$WORK/plugins"

cat > "$WORK/actraild.patch.toml" <<EOF
[control]
socket_path = "$WORK/run/control.sock"
pid_file = "$WORK/run/actraild.pid"
log_path = "$WORK/log/actraild.log"

[storage.sqlite]
path = "$WORK/data/actrail.sqlite"

[storage.retention]
enabled = false

[export.snapshot]
directory = "$WORK/data/export"

[payload.tls]
sync_event_socket_path = "$WORK/run/tls-sync.sock"

[cluster.report]
spool_dir = "$WORK/data/cluster-spool"
state_path = "$WORK/data/cluster-report-state.sqlite"

[cluster.center]
root_dir = "$WORK/data/cluster"

[plugins.discovery]
directory = "$WORK/plugins"
EOF

"$BIN/actraild" --config "$WORK/actraild.conf" \
  init -f --patch "$WORK/actraild.patch.toml"
"$BIN/actraild" --config "$WORK/actraild.conf" stop
"$BIN/actrailctl" --config "$WORK/actraild.conf" clean
"$BIN/actraild" --config "$WORK/actraild.conf" start
```

### 预期结果

安装包四件套（manifest、wasm、两个 payload schema）存在；四条 AcTrail 命令全部成功，daemon 打印 pid 与 control
socket；SQLite 与日志均位于 `/tmp/actrail-frequent-alert-manual`。

## 步骤3：加载插件

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$REPO/examples/plugins/wasm-legacy/tool-frequent-failure-alert/plugin.toml" \
  --plugin-config "$E2E_CONFIG" \
  --instance manual.frequent-alert \
  --grant alert-write
```

### 预期结果

输出 `loaded instance=manual.frequent-alert`、`warnings=none`。

> 测试专用配置把 `tool_scope` 设为 `agent_children`、`parent_scope` 设为
> `any`：无 LLM 的原始命令也能按可执行文件名（如 `ls`）归一到工具维度；
> 真实 Agent 轮仍优先使用 `llm.response.tool_calls_json` 中的 LLM 工具名
> （如 `bash`）。生产默认配置为 `llm_and_mcp` + `agent_child`，只统计
> LLM/宿主回填的工具执行。

## 步骤4：混合正向轮（三次成功与三次失败触发告警）

### 手动指令

```bash
FAILURE_MARKER="TOOL_FREQUENT_ALERT_MIXED_$(date +%s%N)"
FAILURE_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$FAILURE_MARKER" -- \
    bash -c 'ls /etc/hostname; ls /etc/hostname; ls /etc/hostname; ls /actrail-missing-frequent-a; ls /actrail-missing-frequent-b; ls /actrail-missing-frequent-c' \
    2>&1
)"
printf '%s\n' "$FAILURE_OUTPUT"
TRACE_ID="$(
  printf '%s\n' "$FAILURE_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
test -n "$TRACE_ID"

for _ in $(seq 1 30); do
  ALERT_COUNT="$(
    sqlite3 "$WORK/data/actrail.sqlite" \
      "SELECT count(*) FROM alerts a
       JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
       WHERE a.trace_id = $TRACE_ID AND d.definition_key = 'frequent-failure'"
  )"
  test "$ALERT_COUNT" -ge 1 && break
  sleep 1
done
test "$ALERT_COUNT" -eq 1

sqlite3 -header -column "$WORK/data/actrail.sqlite" "
SELECT a.alert_id, a.trace_id, d.title, d.severity_code,
       json_extract(a.payload_json, '$.failure_count') AS failures,
       json_extract(a.payload_json, '$.total_count') AS total,
       json_extract(a.payload_json, '$.failure_rate') AS failure_rate,
       json_extract(a.payload_json, '$.tool_name') AS tool_name,
       json_extract(a.payload_json, '$.failure_type') AS failure_type,
       json_extract(a.payload_json, '$.exit_status') AS exit_status,
       json_extract(a.payload_json, '$.threshold.min_failure_count') AS min_count,
       json_extract(a.payload_json, '$.window.start_ms') AS window_start,
       json_extract(a.payload_json, '$.window.end_ms') AS window_end,
       json_extract(a.payload_json, '$.failure_breakdown') AS breakdown
FROM alerts a
JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
WHERE a.trace_id = $TRACE_ID AND d.definition_key = 'frequent-failure';"
```

### 预期结果

六次执行使用同一个工具名 `ls`。恰好一条告警；`failures=3`、`total=6`、
`failure_rate=0.5`、`tool_name=ls`、
`failure_type=runtime_error`、`exit_status=2`、`min_count=3`；`window_start`
与 `window_end` 为毫秒时间戳；`alert_definitions` 中定义的生产者为
`actrail.tool-frequent-failure-alert`，severity_code 为 4（High）。

可选验证脱敏策略：把 `$E2E_CONFIG` 的 `desensitization.mode` 改为
`sanitized` 后重跑本步骤，`summary` 字段会输出脱敏后的失败摘要
（截断 + 密钥片段替换），且 payload 始终不含 `command.line` 等工具参数
明文。

## 步骤5：低于阈值轮（失败次数不足不告警）

### 手动指令

```bash
LOW_MARKER="TOOL_FREQUENT_ALERT_LOW_$(date +%s%N)"
LOW_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$LOW_MARKER" -- \
    bash -c 'ls /actrail-missing-insufficient-a; ls /actrail-missing-insufficient-b' \
    2>&1
)"
printf '%s\n' "$LOW_OUTPUT"
LOW_TRACE_ID="$(
  printf '%s\n' "$LOW_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
sqlite3 "$WORK/data/actrail.sqlite" \
  "SELECT count(*) FROM alerts a
   JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
   WHERE a.trace_id = $LOW_TRACE_ID AND d.definition_key = 'frequent-failure';"
```

### 预期结果

两条失败低于窗口阈值 3，查询结果为 `0`。窗口聚合不依赖“连续”，成功事件
不清零失败计数（只计入失败率分母），因此本插件的“低于阈值”语义与连续失败
插件的“成功清零”语义不同。

## 步骤6：冷却矩阵（超过阈值的失败不重复告警）

### 手动指令

```bash
COOLDOWN_MARKER="TOOL_FREQUENT_ALERT_COOLDOWN_$(date +%s%N)"
COOLDOWN_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$COOLDOWN_MARKER" -- \
    bash -c 'ls /actrail-missing-cooldown-a; ls /actrail-missing-cooldown-b; ls /actrail-missing-cooldown-c; ls /actrail-missing-cooldown-d; ls /actrail-missing-cooldown-e' \
    2>&1
)"
printf '%s\n' "$COOLDOWN_OUTPUT"
COOLDOWN_TRACE_ID="$(
  printf '%s\n' "$COOLDOWN_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
sqlite3 "$WORK/data/actrail.sqlite" \
  "SELECT count(*) FROM alerts a
   JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
   WHERE a.trace_id = $COOLDOWN_TRACE_ID AND d.definition_key = 'frequent-failure';"

"$BIN/actrailviewer" --config "$WORK/actraild.conf" \
  --output-format json actions --trace-id "$COOLDOWN_TRACE_ID" |
  jq '[.actions[] | select(.kind == "command.invocation")] | length'
```

### 预期结果

五次失败仍只有 `1` 条告警（第三次失败触发后窗口重置，第四、五次失败处于
冷却期被抑制）；viewer 中 `command.invocation` 数量为 6（bash + 5 条
`ls`），证明额外失败确实发生。

## 步骤7：从安装包加载插件并验证

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.frequent-alert

"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$PLUGIN_ROOT/tool-frequent-failure-alert/tool-frequent-failure-alert.plugin.toml" \
  --plugin-config "$E2E_CONFIG" \
  --instance manual.frequent-alert-installed \
  --grant alert-write

INSTALLED_MARKER="TOOL_FREQUENT_ALERT_INSTALLED_$(date +%s%N)"
"$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
  --name "$INSTALLED_MARKER" -- \
  bash -c 'ls /actrail-missing-frequent-a; ls /actrail-missing-frequent-b; ls /actrail-missing-frequent-c'

"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.frequent-alert-installed
"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$REPO/examples/plugins/wasm-legacy/tool-frequent-failure-alert/plugin.toml" \
  --plugin-config "$E2E_CONFIG" \
  --instance manual.frequent-alert \
  --grant alert-write
```

### 预期结果

先卸载仓库实例、只保留安装包实例，避免两个实例对同一批事件各提交一条重复
告警（宿主 `alert_deduplication_keys` 本身也保证幂等）；安装包实例加载
成功，该轮 trace 恰好落库一条告警，证明安装产物可用；随后恢复仓库实例供
真实 Agent 轮使用。

## 步骤8：真实 Agent 轮（可选）

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.frequent-alert
"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$REPO/examples/plugins/wasm-legacy/tool-frequent-failure-alert/plugin.toml" \
  --plugin-config "$AGENT_CONFIG" \
  --instance manual.frequent-alert \
  --grant alert-write

if command -v opencode >/dev/null 2>&1; then
  AGENT_BIN="$(command -v opencode)"
  AGENT_NAME=opencode
  AGENT_COMMAND=("$AGENT_BIN" run)
elif command -v codex >/dev/null 2>&1; then
  AGENT_BIN="$(command -v codex)"
  AGENT_NAME=codex
  AGENT_COMMAND=(
    "$AGENT_BIN" exec --ephemeral
    -m "${CODEX_E2E_MODEL:-gpt-5.5}"
    -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}"
  )
else
  AGENT_NAME=
fi

if [ -n "$AGENT_NAME" ]; then
  AGENT_MARKER="TOOL_FREQUENT_ALERT_AGENT_$(date +%s%N)"
  AGENT_PROMPT="请分三条独立命令依次执行，每条命令只检查一个路径，不要合并命令，也不要跳过：ls /actrail-missing-agent-a.txt、ls /actrail-missing-agent-b.txt、ls /actrail-missing-agent-c.txt。执行后原样报告每条命令的输出。"
  AGENT_OUTPUT="$(
    "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
      --name "$AGENT_MARKER" -- \
      "${AGENT_COMMAND[@]}" "$AGENT_PROMPT" 2>&1
  )"
  printf '%s\n' "$AGENT_OUTPUT"
  AGENT_TRACE_ID="$(
    printf '%s\n' "$AGENT_OUTPUT" |
      sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
  )"
  test -n "$AGENT_TRACE_ID"

  for _ in $(seq 1 30); do
    AGENT_ALERT_COUNT="$(
      sqlite3 "$WORK/data/actrail.sqlite" \
        "SELECT count(*) FROM alerts a
         JOIN alert_definitions d
           ON a.alert_definition_id = d.alert_definition_id
         WHERE a.trace_id = $AGENT_TRACE_ID
           AND d.definition_key = 'frequent-failure'"
    )"
    test "$AGENT_ALERT_COUNT" -ge 1 && break
    sleep 1
  done

  AGENT_ACTIONS="$(
    "$BIN/actrailviewer" --config "$WORK/actraild.conf" \
      --output-format json actions --trace-id "$AGENT_TRACE_ID"
  )"
  AGENT_LS_COUNT="$(
    printf '%s' "$AGENT_ACTIONS" |
      jq '[.actions[] |
            select(.kind == "command.invocation") |
            select(
              (.attributes["process.executable"] // "" | endswith("/ls")) or
              (.attributes["command.line"] // "" | startswith("ls "))
            )] | length'
  )"
  AGENT_TOOL_CALL_COUNT="$(
    printf '%s' "$AGENT_ACTIONS" |
      jq '[.actions[] |
            select(.kind == "llm.response") |
            .attributes["llm.response.tool_calls_json"]? |
            select(type == "string" and length > 0) |
            fromjson? |
            .[] |
            select(type == "object")] | length'
  )"

  test "$AGENT_LS_COUNT" -ge 3
  test "$AGENT_TOOL_CALL_COUNT" -ge 3
  test "$AGENT_ALERT_COUNT" -eq 1

  sqlite3 -header -column "$WORK/data/actrail.sqlite" \
    "SELECT a.alert_id, a.trace_id, d.definition_key,
            json_extract(a.payload_json, '$.tool_name') AS tool,
            json_extract(a.payload_json, '$.failure_count') AS failures,
            json_extract(a.payload_json, '$.total_count') AS total,
            json_extract(a.payload_json, '$.failure_rate') AS rate,
            json_extract(a.payload_json, '$.evidence_action_ids') AS evidence
     FROM alerts a
     JOIN alert_definitions d
       ON a.alert_definition_id = d.alert_definition_id
     WHERE a.trace_id = $AGENT_TRACE_ID
       AND d.definition_key = 'frequent-failure';"
else
  echo "no usable agent binary; skipping real-agent round"
fi
```

### 预期结果

有可用 Agent 时（自动回测选择具备工具能力的 `opencode` 或 `codex`，且优先使用
`OPENCODE_E2E_BINARY` / `CODEX_E2E_BINARY` 等环境变量），Agent 执行三条独立
失败命令后，`AGENT_LS_COUNT >= 3`、`AGENT_TOOL_CALL_COUNT >= 3`，并且该
trace 恰好落库一条告警。真实 Agent 通过 `llm.response` 的
告警 payload 必须是 `failure_count=3`、`total_count=3`、
`failure_rate=1.0`，证据中至少有三条命令和三条退出动作。自动回测会在最终
`real-agent` 结果中打印完整的紧凑格式 `payload_json`；隔离数据库会在
`cleanup` 阶段删除，因此请从该输出查看本轮真实告警内容。
`tool_calls_json` 返回工具名（如 `bash`），插件优先用工具参数提示匹配、
流式更新按 tool call ID 去重；队列中的旧提示不匹配当前命令时，当前命令继续
进入延迟缓冲，等待它自己的响应到达后回放，因此三次顺序调用都能累计。
无提示时按 FIFO 归一到后续非嵌套命令，聚合键是 LLM 工具名而不是命令行，
因此不会因为每条 bash 命令不同而产生高基数。自动回测最多执行两次：若 Agent 未实际执行失败
命令（trace 中找不到三条 `ls` 调用），判为 `SKIPPED` 不报错；若执行了失败
命令但无告警，或有告警却没有至少三条 LLM 工具调用，判为 `FAILED`。统一
提示词不依赖 Agent 的具体命令格式。

## 步骤9：清理

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.frequent-alert-installed
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.frequent-alert
"$BIN/actraild" --config "$WORK/actraild.conf" stop
"$BIN/actrailctl" --config "$WORK/actraild.conf" clean
rm -rf -- /tmp/actrail-frequent-alert-manual
```

### 预期结果

两个插件实例卸载成功，daemon 停止，隔离目录被删除；系统默认
`/var/lib/actrail` 未受影响。

# 覆盖范围与非目标

本用例覆盖：插件加载与授权、告警定义注册、告警落库与 payload 字段
（工具名/失败类型/退出状态/失败分布/失败计数/窗口范围/证据列表）、证据唯一性
（去重）、同一工具成功/失败混合统计（`failure_count=3`、`total_count=6`、
`failure_rate=0.5`）、窗口阈值（低于阈值不告警）、冷却抑制（冷却矩阵）、
`install-release.sh` 安装包（manifest、wasm、两个 payload schema）与安装包
加载，以及可选的真实 Agent 端到端验证（真实模型对话、三次工具执行、
`llm.response.tool_calls_json` 计数、LLM 工具名归并以及告警落库）。

本用例不覆盖：

- Web 插件管理交互（发现、加载、配置、状态页面），由独立 Web 插件回归负责；
- 跨 batch 重投去重的注入验证（真实链路不会重投同一动作，以
  `evidence_action_ids` 唯一性和每轮恰好一条告警覆盖）；
- 失败率阈值（`trigger_mode=rate`）、告警风暴、长时冷却参数矩阵或多种
  阈值组合；
- `desensitization.mode=sanitized` 的关键词替换与截断细节未自动化（默认
  `category_only` 只校验 payload 不含 `summary` 与工具参数明文），手动步骤 4
  提供了切换配置后的可选验证；
- MCP 工具（`mcp.tool_call`）与策略决策（`enforcement.decision`）失败聚合
  的注入验证，真实链路下由对应语义动作驱动；
- 多插件并发、容器隔离等重型场景。

关键设计事实：

- `process.exit` / `agent.exit` 是 export-only 动作，不进入 SQLite，因此
  回测从告警 payload 的证据断言而非存储断言；
- 宿主导出时为每个语义动作注入 `process.id`（插件可见、不落库），该注入由
  `crates/core/plugin_wasm_runtime/src/component_observation/wire.rs` 的单元
  测试守护；
- 聚合键为 `(trace_id, tool_name)`，窗口内记录 `(failure_type, exit_status)`
  分布（`failure_breakdown`）；成功事件不清零失败计数，只计入失败率分母；
- 工具维度只取 LLM 工具名 / MCP 工具名 / 可执行文件名，`command.line`
  永远不是聚合键（Shell 矩阵轮依赖可执行文件名 `ls`，真实 Agent 轮依赖
  LLM 工具名 `bash`）；
- 告警触发后窗口重置，冷却期内同聚合键不重复告警；宿主按
  `(trace_id, alert_definition_id, deduplication_key)` 幂等落库；
- 退出码为 0 的成功进程在 `process.exit` 上表现为 `status=unknown` 且无
  `process.exit_code`，插件默认视为成功（混合正向轮通过 `total_count=6` 和
  `failure_rate=0.5` 端到端验证该行为）；
- 无法判定成败的事件（如 `status=unknown` 且配置不视为成功）默认跳过，
  不伪造失败告警；`indeterminate_handling=diagnostic` 时提交
  `indeterminate-result` 信息级告警。
