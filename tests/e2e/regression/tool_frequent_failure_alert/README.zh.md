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
- 真实 Agent 轮用统一提示词驱动 Agent 执行三条失败命令，检查工具调用证据和告警。

## 判定语义

- AcTrail 自身产物缺失或不可执行（`actraild`、`actrailctl`、
  `actrailviewer`、release 安装的插件包）判定为 `FAILED`；
- 真实 Agent 缺失或不可用（`opencode`、`codex` 均未找到）时，真实 Agent
  子项判定为 `SKIPPED`；
- 真实 Agent 轮无告警且未观测到三条失败命令时，判定为 `SKIPPED`；
- 已观测到三条失败命令但无告警，或已有告警但失败命令、LLM 工具调用任一
  不足三条时，判定为 `FAILED`。

## 回测组成

| 回测层次 | 运行配置 | 执行主体 | 自动断言 |
| --- | --- | --- | --- |
| 直接命令回测 | `tool-frequent-failure-alert.e2e.config.json`（`agent_children + any`） | `actrailctl launch -- bash -c ...` | 同一 `ls` 工具三次成功加三次失败后告警并得到 `total_count=6`、`failure_rate=0.5`，低于阈值不告警、冷却只告警一次、payload/证据与安装包 |
| 真实 Agent 对话回测 | `tool-frequent-failure-alert.agent.e2e.config.json`（`llm_and_mcp + agent_child`） | OpenCode `run` 或 Codex `exec` 与模型真实对话后调用工具 | 三次失败 `ls`、至少三条 `llm.response.tool_calls_json` 工具调用、告警及命令/退出证据 |

真实 Agent 轮启动 Agent CLI 与模型对话，从同一 trace 的 LLM 响应、命令和
进程退出动作中核对三次工具调用。

# Quick Run

混合正向轮通过 `bash -c 'exec ls <path>'` 执行三条成功命令和三条失败命令。
`terminal-command-registration` 核对同一逻辑进程的 bash 阶段终态和 ls 阶段，
告警中的命令证据均属于实际 ls，并包含三个失败 ls 的 action ID。
该轮预期失败数为 3、总数为 6、失败率为 0.5，生成一条告警。

在仓库根目录以 root 运行回归。

```bash
sudo -E python3.11 tests/e2e/regression/test_all.py \
  --case tool_frequent_failure_alert
```

公共 runner 会先执行 `scripts/install-release.sh` 构建并安装官方插件包。用例将
`${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/` 中的 WASM 与仓库的 manifest、
schema 组装到隔离工作目录，再启动真实 `actraild`。加载插件后依次跑混合正向、
低于阈值、冷却、安装包和可选真实 Agent 轮次。首次运行的编译耗时取决于本机缓存。

只运行该 case 并显示详细过程：

```bash
sudo -E python3.11 \
  tests/e2e/regression/tool_frequent_failure_alert/run_e2e.py
```

# 步骤摘要

1. 检查 release 二进制（`actraild`、`actrailctl`、`actrailviewer`）与
   root/eBPF 测试权限。
2. 断言 `scripts/install-release.sh` 已把插件安装到
   `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/tool-frequent-failure-alert/`
   （manifest、wasm、两个 payload schema）。
3. 在隔离目录组装插件包，初始化、清理并启动 AcTrail。
4. 从隔离目录加载插件，携带测试专用配置
   `tool-frequent-failure-alert.e2e.config.json`（`tool_scope=agent_children`、
   `parent_scope=any`），并授予 `alert-write`。
5. 混合正向轮：同一 `ls` 工具先成功三次、再失败三次，断言定义注册、恰好
   一条告警，并检查 `failure_count=3`、`total_count=6`、
   `failure_rate=0.5` 以及 payload 字段与
   证据（`evidence_action_ids` 唯一且同时含 `:command.invocation` 与
   `:process.exit` 动作 id）正确。
6. 低于阈值轮：同一 trace 只有两条失败 `ls`，断言无告警。
7. 冷却矩阵：单 trace 连续五条失败命令，断言告警数为 1，并核对存储的
   `command.invocation` 数量。
8. 断言隔离目录的插件实例 `observed_records > 0`。
9. 安装包轮先卸载隔离目录的插件实例，再加载安装目录的插件并跑一轮正向验证。
   完成后卸载安装包实例，并从隔离目录重新加载插件。
10. 真实 Agent 轮（可选）：切换到
    `tool-frequent-failure-alert.agent.e2e.config.json`
    （`tool_scope=llm_and_mcp`、`parent_scope=agent_child`），再探测
    `opencode`/`codex`，用统一提示词驱动其执行三条失败命令，覆盖工具命令/
    退出早于流式 `llm.response` 最终化的延迟归因，以及重复 tool call 去重和
    顺序工具调用；最多执行两次。告警 payload 满足 `failure_count=3`、
    `total_count=3`、`failure_rate=1.0`，并至少包含三组命令/退出证据。
    轮次结果按[判定语义](#判定语义)确定。
11. 卸载插件、停止 daemon、清理。

# 手动测试

按[插件编译说明](../../../../examples/plugins/wasm-legacy/tool-frequent-failure-alert/README.zh.md#编译)生成示例目录中的 WASM。
在仓库根目录用同一个 root shell 执行以下命令。
手动测试工作目录为 `/tmp/actrail-frequent-alert-manual`，命令通过该目录下的
`actraild.conf` 操作隔离实例。

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
E2E_CONFIG="$REPO/tests/e2e/regression/tool_frequent_failure_alert/tool-frequent-failure-alert.e2e.config.json"

AGENT_CONFIG="$REPO/tests/e2e/regression/tool_frequent_failure_alert/tool-frequent-failure-alert.agent.e2e.config.json"
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
directory = ["$WORK/plugins"]
EOF

"$BIN/actrailctl" --config "$WORK/actraild.conf" \
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
> 真实 Agent 轮优先使用 `llm.response.tool_calls_json` 中的 LLM 工具名
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

两条失败低于窗口阈值 3，查询结果为 `0`。窗口内的成功事件计入调用总数，
失败率按失败数除以调用总数计算。

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

告警数为 `1`：第三次失败触发告警并重置窗口，第四、五次失败位于冷却期。
viewer 中 `command.invocation` 数量为 6（bash + 5 条 `ls`）。

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

安装包实例加载成功，该轮 trace 落库一条告警。完成后卸载安装包实例，加载
仓库实例供真实 Agent 轮使用。

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

有可用 Agent 时（自动回测选择 `opencode` 或 `codex`，且优先使用
`OPENCODE_E2E_BINARY` / `CODEX_E2E_BINARY` 等环境变量），Agent 执行三条独立
失败命令后，`AGENT_LS_COUNT >= 3`、`AGENT_TOOL_CALL_COUNT >= 3`，并且该
trace 落库一条告警。告警 payload 为 `failure_count=3`、`total_count=3`、
`failure_rate=1.0`，证据中至少有三条命令和三条退出动作。自动回测会在最终
`real-agent` 结果中打印完整的紧凑格式 `payload_json`；隔离数据库会在
`cleanup` 阶段删除，告警内容可从该输出查看。

工具名来自 `llm.response.tool_calls_json`（如 `bash`）。插件优先按工具参数提示
关联命令，流式更新按 tool call ID 去重；未匹配的命令进入延迟缓冲，收到匹配
响应后回放统计。无参数提示时，按 FIFO 关联后续非嵌套命令。

自动回测最多执行两次，轮次结果按[判定语义](#判定语义)确定。

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

两个插件实例卸载成功，daemon 停止，隔离目录被删除。

# 覆盖范围

本用例覆盖：插件加载与授权、告警定义注册、告警落库与 payload 字段
（工具名/失败类型/退出状态/失败分布/失败计数/窗口范围/证据列表）、证据唯一性
（去重）、同一工具成功/失败混合统计（`failure_count=3`、`total_count=6`、
`failure_rate=0.5`）、窗口阈值（低于阈值不告警）、冷却抑制（冷却矩阵）、
`install-release.sh` 安装包（manifest、wasm、两个 payload schema）与安装包
加载，以及可选的真实 Agent 端到端验证（真实模型对话、三次工具执行、
`llm.response.tool_calls_json` 计数、LLM 工具名归并以及告警落库）。

`process.exit` / `agent.exit` 通过观测出口传给插件，回测通过告警 payload 的
`evidence_action_ids` 核对退出动作。宿主向插件提供的 `process.id` 为逻辑进程 ID。
