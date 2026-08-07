# Tool Consecutive Failure Alert 回归

## 测试目标

该用例验证 `wasm-legacy/tool-consecutive-failure-alert` 插件的端到端告警链路：

- 插件通过 `alert-write` 授权加载，消费 `command.invocation` / `process.exit`
  语义动作，并按宿主导出时注入的 `process.id` 精确关联；
- 同一 trace 内同一工具连续失败达到阈值（默认 3）后，`consecutive-failure`
  告警定义被注册且告警落库；
- 成功命令（外部命令，退出码 0）清零连续失败计数；
- 超过阈值的持续失败被冷却抑制，不重复告警；
- `scripts/install-release.sh` 安装出的官方插件包可被发现并从安装目录加载；
- 机器上存在真实 Agent 时，用统一提示词驱动 Agent 执行三条失败命令并验证
  告警；没有可用 Agent 时该轮次跳过，不计失败。

## 判定语义

- AcTrail 自身产物缺失或不可执行（`actraild`、`actrailctl`、
  `actrailviewer`、release 安装的插件包）判定为 `FAILED`，不降级为
  `SKIPPED`；
- 真实 Agent 缺失或不可用（`xiaoo`、`pi`、`opencode`、`claude`、
  `codex` 均未找到）时，真实 Agent 子项判定为 `SKIPPED`，不影响整体
  `PASSED`。

# Quick Run

在仓库根目录以 root 执行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case tool_consecutive_failure_alert
```

公共 runner 会先执行 `scripts/install-release.sh`（构建并安装官方插件包），再
创建隔离工作目录，启动真实 `actraild`，加载插件后依次跑负向、正向、重置、
冷却、安装包和可选真实 Agent 轮次。冷缓存机器可能长时间停留在 release、
TLS runtime 或 WASM 插件编译阶段；只要 `cargo`/`rustc` 仍在占用 CPU，就不是
测试卡死。

只运行该 case 并显示详细过程：

```bash
sudo -E python3.11 \
  tests/v2/regression/tool_consecutive_failure_alert/run_e2e.py
```

# 步骤摘要

1. 检查 release 二进制（`actraild`、`actrailctl`、`actrailviewer`）与
   root/eBPF 测试权限。
2. 断言 `scripts/install-release.sh` 已把插件安装到
   `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}/tool-consecutive-failure-alert/`
   （manifest、wasm、payload schema 三件套）。
3. 在隔离目录初始化、清理并启动 AcTrail。
4. 从仓库示例目录加载插件（`--grant alert-write`）。
5. 负向轮：`/bin/true` 连续成功三次，断言无告警。
6. 正向轮：三条失败 `ls`，断言定义注册、恰好一条告警、payload 字段与
   证据（`evidence_action_ids` 唯一且含 `:process.exit` 动作 id）正确。
7. 重置矩阵：失败-失败-同一工具成功（`ls /etc/hostname`）-失败-失败，
   断言成功清零后不再告警。
8. 冷却矩阵：单 trace 连续五条失败命令，断言仍只有一条告警，且存储的
   `command.invocation` 数量证明额外失败确实发生（冷却抑制重复告警）。
9. 断言仓库实例 `observed_records > 0`。
10. 安装包轮：临时卸载仓库实例，仅加载安装目录实例跑一轮正向（同一时刻
    只保留一个实例，避免两个实例对同一批事件各提交一条重复告警），断言
    安装产物可用；完成后恢复仓库实例。
11. 真实 Agent 轮（可选）：按顺序选择第一个找到的 agent 二进制
    （`xiaoo`/`pi`/`opencode`/`claude`/`codex`），不再逐个真实探测；设置
    `ACTRAIL_TEST_AGENT_PROBE_ALL=1` 可恢复逐候选真实探测。用统一提示词
    驱动选中 agent 执行三条失败命令，最多重试两次。无可用 Agent 或 Agent
    未实际执行失败命令时 `SKIPPED`；Agent 执行了失败命令但无告警落库则
    `FAILED`。
12. 卸载插件、停止 daemon、清理。

# 手动测试

以下命令完整复现自动测试，均从仓库根目录、在同一个 root shell 中执行。
手动测试使用隔离的 `/tmp/actrail-tool-alert-manual` 工作目录，不读取或清理
系统默认的 `/var/lib/actrail` 数据。

## 步骤1：检查测试前提并构建 release

### 手动指令

```bash
test "$(id -u)" -eq 0
command -v python3 >/dev/null
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
test -f examples/plugins/wasm-legacy/tool-consecutive-failure-alert/plugin.toml

bash scripts/install-release.sh
```

### 预期结果

当前用户为 root；三个测试所需二进制均可执行；插件 manifest 存在；安装脚本
成功并把插件安装到 `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}`。

## 步骤2：确认安装包并创建隔离配置启动 daemon

### 手动指令

```bash
PLUGIN_ROOT="${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}"
test -f "$PLUGIN_ROOT/tool-consecutive-failure-alert/tool-consecutive-failure-alert.plugin.toml"
test -f "$PLUGIN_ROOT/tool-consecutive-failure-alert/actrail_tool_consecutive_failure_alert.wasm"
test -f "$PLUGIN_ROOT/tool-consecutive-failure-alert/alert-schema.json"

REPO="$(pwd -P)"
BIN="$REPO/target/release"
WORK=/tmp/actrail-tool-alert-manual

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

安装包三件套存在；四条 AcTrail 命令全部成功，daemon 打印 pid 与 control
socket；SQLite 与日志均位于 `/tmp/actrail-tool-alert-manual`。

## 步骤3：加载插件

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$REPO/examples/plugins/wasm-legacy/tool-consecutive-failure-alert/plugin.toml" \
  --instance manual.tool-alert \
  --grant alert-write
```

### 预期结果

输出 `loaded instance=manual.tool-alert`、`warnings=none`。

## 步骤4：负向轮（成功命令不告警）

### 手动指令

```bash
SUCCESS_MARKER="TOOL_ALERT_SUCCESS_$(date +%s%N)"
"$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
  --name "$SUCCESS_MARKER" -- \
  bash -c '/bin/true; /bin/true; /bin/true'
```

### 预期结果

输出唯一 `trace trace-N entered Active`；对应 trace 在 `alerts` 表中无记录。

## 步骤5：正向轮（三条失败命令触发告警）

### 手动指令

```bash
FAILURE_MARKER="TOOL_ALERT_FAILURE_$(date +%s%N)"
FAILURE_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$FAILURE_MARKER" -- \
    bash -c 'ls /actrail-missing-consecutive-a; ls /actrail-missing-consecutive-b; ls /actrail-missing-consecutive-c' \
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
       WHERE a.trace_id = $TRACE_ID AND d.definition_key = 'consecutive-failure'"
  )"
  test "$ALERT_COUNT" -ge 1 && break
  sleep 1
done
test "$ALERT_COUNT" -eq 1

sqlite3 -header -column "$WORK/data/actrail.sqlite" "
SELECT a.alert_id, a.trace_id, d.title, d.severity_code,
       json_extract(a.payload_json, '$.consecutive_failures') AS failures,
       json_extract(a.payload_json, '$.threshold') AS threshold,
       json_extract(a.payload_json, '$.tool_name') AS tool_name,
       json_extract(a.payload_json, '$.failure_summary') AS summary
FROM alerts a
JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
WHERE a.trace_id = $TRACE_ID AND d.definition_key = 'consecutive-failure';"
```

### 预期结果

恰好一条告警；`failures=3`、`threshold=3`、`tool_name=ls`、`summary` 形如
`exit code 2`；`alert_definitions` 中定义的生产者为
`tool-consecutive-failure-alert`，severity_code 为 4（High）。

## 步骤6：重置矩阵（成功清零计数）

### 手动指令

```bash
RESET_MARKER="TOOL_ALERT_RESET_$(date +%s%N)"
RESET_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$RESET_MARKER" -- \
    bash -c 'ls /actrail-missing-reset-a; ls /actrail-missing-reset-b; ls /etc/hostname; ls /actrail-missing-reset-c; ls /actrail-missing-reset-d' \
    2>&1
)"
printf '%s\n' "$RESET_OUTPUT"
RESET_TRACE_ID="$(
  printf '%s\n' "$RESET_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
sqlite3 "$WORK/data/actrail.sqlite" \
  "SELECT count(*) FROM alerts a
   JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
   WHERE a.trace_id = $RESET_TRACE_ID AND d.definition_key = 'consecutive-failure';"
```

### 预期结果

两次失败后被同一工具的成功调用（`ls /etc/hostname`，退出码 0）清零，随后
两次失败不足以再次触发，查询结果为 `0`。插件按 `(trace, tool)` 独立计数，
成功只清零同一工具的计数，因此成功命令必须是 `ls` 才能重置 `ls` 的计数。

## 步骤7：冷却矩阵（超过阈值的失败不重复告警）

### 手动指令

```bash
COOLDOWN_MARKER="TOOL_ALERT_COOLDOWN_$(date +%s%N)"
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
   WHERE a.trace_id = $COOLDOWN_TRACE_ID AND d.definition_key = 'consecutive-failure';"

"$BIN/actrailviewer" --config "$WORK/actraild.conf" \
  --output-format json actions --trace-id "$COOLDOWN_TRACE_ID" |
  jq '[.actions[] | select(.kind == "command.invocation")] | length'
```

### 预期结果

五次失败仍只有 `1` 条告警（第三次失败触发后，冷却抑制第四、五次失败重复
告警）；viewer 中 `command.invocation` 数量为 6（bash + 5 条 `ls`），证明额外
失败确实发生。

## 步骤8：从安装包加载插件并验证

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.tool-alert

"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$PLUGIN_ROOT/tool-consecutive-failure-alert/tool-consecutive-failure-alert.plugin.toml" \
  --instance manual.tool-alert-installed \
  --grant alert-write

INSTALLED_MARKER="TOOL_ALERT_INSTALLED_$(date +%s%N)"
"$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
  --name "$INSTALLED_MARKER" -- \
  bash -c 'ls /actrail-missing-consecutive-a; ls /actrail-missing-consecutive-b; ls /actrail-missing-consecutive-c'

"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.tool-alert-installed
"$BIN/actraild" --config "$WORK/actraild.conf" plugin load \
  --manifest "$REPO/examples/plugins/wasm-legacy/tool-consecutive-failure-alert/plugin.toml" \
  --instance manual.tool-alert \
  --grant alert-write
```

### 预期结果

先卸载仓库实例、只保留安装包实例，避免两个实例对同一批事件各提交一条重复
告警；安装包实例加载成功，该轮 trace 恰好落库一条告警，证明安装产物可用；
随后恢复仓库实例供真实 Agent 轮使用。

## 步骤9：真实 Agent 轮（可选）

### 手动指令

```bash
if command -v opencode >/dev/null 2>&1; then
  AGENT_BIN="$(command -v opencode)"
  AGENT_NAME=opencode
elif command -v codex >/dev/null 2>&1; then
  AGENT_BIN="$(command -v codex)"
  AGENT_NAME=codex
elif command -v claude >/dev/null 2>&1; then
  AGENT_BIN="$(command -v claude)"
  AGENT_NAME=claude
else
  AGENT_NAME=
fi

if [ -n "$AGENT_NAME" ]; then
  AGENT_MARKER="TOOL_ALERT_AGENT_$(date +%s%N)"
  "$BIN/actrailctl" --config "$WORK/actraild.conf" launch \
    --name "$AGENT_MARKER" -- \
    "$AGENT_BIN" run "请分三条独立命令依次执行，每条命令只检查一个路径，不要合并命令，也不要跳过：ls /actrail-missing-agent-a.txt、ls /actrail-missing-agent-b.txt、ls /actrail-missing-agent-c.txt。执行后原样报告每条命令的输出。"
else
  echo "no usable agent binary; skipping real-agent round"
fi
```

### 预期结果

有可用 Agent 时（自动回测还额外支持 `xiaoo`、`pi`，且优先使用
`XIAOO_E2E_BINARY` / `OPENCODE_E2E_BINARY` 等环境变量），Agent 执行三条独立
失败命令后该 trace 落库一条告警。自动回测最多重试两次：若 Agent 未实际执行
失败命令（trace 中找不到三条 `ls` 调用），判为 `SKIPPED` 不报错；若执行了
失败命令但无告警，判为 `FAILED`。统一提示词不依赖 Agent 的具体命令格式。

## 步骤10：清理

### 手动指令

```bash
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.tool-alert-installed
"$BIN/actraild" --config "$WORK/actraild.conf" plugin unload \
  --instance manual.tool-alert
"$BIN/actraild" --config "$WORK/actraild.conf" stop
"$BIN/actrailctl" --config "$WORK/actraild.conf" clean
rm -rf -- /tmp/actrail-tool-alert-manual
```

### 预期结果

两个插件实例卸载成功，daemon 停止，隔离目录被删除；系统默认
`/var/lib/actrail` 未受影响。

# 覆盖范围与非目标

本用例覆盖：插件加载与授权、告警定义注册、告警落库与 payload 字段、证据
唯一性（去重）、成功清零（重置矩阵）、冷却抑制（冷却矩阵）、
`install-release.sh` 安装包三件套与安装包加载，以及可选的真实 Agent 端到端
验证。

本用例不覆盖：

- Web 插件管理交互（发现、加载、配置、状态页面），由独立 Web 插件回归负责；
- 跨 batch 重投去重的注入验证（真实链路不会重投同一动作，以
  `evidence_action_ids` 唯一性和每轮恰好一条告警覆盖）；
- 告警风暴、长时冷却参数矩阵或多种阈值组合；
- 多插件并发、容器隔离等重型场景。

关键设计事实：

- `process.exit` / `agent.exit` 是 export-only 动作，不进入 SQLite，因此
  回测从告警 payload 的证据断言而非存储断言；
- 宿主导出时为每个语义动作注入 `process.id`（插件可见、不落库），该注入由
  `crates/core/plugin_wasm_runtime/src/component_observation/wire.rs` 的单元
  测试守护；
- 计数按 `(trace, tool)` 独立维护，同一工具的成功调用清零该工具计数；
  不同工具的成功不影响当前工具计数（重置矩阵使用 `ls /etc/hostname`
  验证 `ls` 的计数清零）；
- 退出码为 0 的成功进程在 `process.exit` 上表现为 `status=unknown` 且无
  `process.exit_code`，插件默认视为成功（重置矩阵依赖该行为）。
