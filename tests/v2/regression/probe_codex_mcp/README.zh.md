# Probe Codex MCP

该测例让真实 Codex 会话调用仓库自带的本地 stdio MCP 工具，并验证 AcTrail
生成的完整语义图。

## 运行

从仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/probe_codex_mcp/run_e2e.py
```

或通过总入口执行：

```bash
sudo -E python3 tests/v2/regression/test_all.py --case probe_codex_mcp
```

runner 会先刷新 release 产物。Codex 缺失或外部可用性检查失败时测例为
`SKIPPED`；AcTrail、MCP probe、固定断言或清理失败时测例为 `FAILED`。
测例在 case work directory 内生成独立 operator config，隔离 control socket、
PID、日志、SQLite、TLS-sync socket、export 和 plugin 路径，不会停止默认或其他
显式配置启动的 daemon。

## 验证范围

测例为本轮 MCP server 注入随机 marker，并要求以下条件全部成立：

- Codex 输出 final marker；
- probe JSONL 中恰好有一次 `tools/call`、一次真实执行及对应成功响应；
- `tools/list` 响应大于 4095-byte stdio capture ABI 上限，trace 中持久化
  `mcp_stdio_candidate_stream_discarded` / `candidate_truncated` 诊断，但后续完整
  stdin `tools/call` 仍完成 Candidate 准入；
- trace 最终为 `Exited/Clean`；
- 恰好生成一个 `mcp.tool_call` 以及 `mcp.request`、`mcp.response`、
  `mcp.stdout`、`mcp.stdin` 四个子 action；
- 所有 action 为 `success/complete`，server、tool 和 transport 精确匹配；
- action 引用及五类层级 link 唯一，link 为 `valid=true`、
  `confidence=observed`；
- tool call 关联真实 `llm.response`，并包含精确的 LLM tool-call id/name；
- 默认 `stdout_storage_mode=drop` 下不持久化 stdout payload，但必须存在
  stdin payload，所有 payload evidence 都引用已持久化 segment。

## 手动测试

配置仓库自带的 stdio MCP probe，启动隔离的 AcTrail daemon，运行
一次真实 Codex 调用，再用 viewer 核验结果。
所有步骤都只覆盖本地 stdio MCP，不会配置或调用远程 MCP。请在仓库根目录的
同一个 Bash 中依次执行，以保留变量、数组和 `EXIT` trap。

### 步骤 1：检查前提和 Codex 外部可用性

#### 手动指令

```bash
set -euo pipefail

test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
command -v jq >/dev/null
command -v rg >/dev/null

REGRESSION_PYTHON="${ACTRAIL_TEST_PYTHON:-python3}"
"$REGRESSION_PYTHON" -c \
  'import sys; assert sys.version_info >= (3, 10), sys.version'

CODEX_BIN="${CODEX_E2E_BINARY:-$(command -v codex || true)}"
test -n "$CODEX_BIN"
test -x "$CODEX_BIN"
CODEX_RESOLVED_BIN="$(readlink -f "$CODEX_BIN")"
CODEX_AUTH_USER_HOME="$HOME"
CODEX_AUTH_CONFIG_HOME="${CODEX_HOME:-$CODEX_AUTH_USER_HOME/.codex}"
case "$CODEX_RESOLVED_BIN" in
  */.codex/*)
    CODEX_AUTH_USER_HOME="${CODEX_RESOLVED_BIN%%/.codex/*}"
    CODEX_AUTH_CONFIG_HOME="$CODEX_AUTH_USER_HOME/.codex"
    ;;
esac
test -d "$CODEX_AUTH_CONFIG_HOME"

CODEX_PREFLIGHT_OUTPUT="$(
  env \
    "HOME=$CODEX_AUTH_USER_HOME" \
    "CODEX_HOME=$CODEX_AUTH_CONFIG_HOME" \
    "$CODEX_BIN" exec \
    --ephemeral \
    -m "${CODEX_E2E_MODEL:-gpt-5.5}" \
    -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}" \
    'Reply with exactly "CODEX_MCP_PREFLIGHT_OK" and nothing else. Do not use tools.' \
    </dev/null
)"
test "$CODEX_PREFLIGHT_OUTPUT" = "CODEX_MCP_PREFLIGHT_OK"
printf '%s\n' "$CODEX_PREFLIGHT_OUTPUT"
```

#### 预期结果

三个 release 二进制均存在，Python 版本满足要求，Codex 成功输出
`CODEX_MCP_PREFLIGHT_OK`。预检使用从 Codex 真实路径解析出的认证目录，与后续
sudo launch 的环境一致。这一步会实际访问配置的 model/provider；仅检查
`codex --version` 不能证明认证和外部服务可用。

### 步骤 2：创建独占目录、stdio MCP 参数和隔离运行时配置

#### 手动指令

```bash
PYTHON_BIN="$(
  "$REGRESSION_PYTHON" -c \
    'import sys; from pathlib import Path; print(Path(sys.executable).resolve())'
)"
PROBE_SCRIPT="$(
  readlink -f tests/v2/common/test_suite_tools/mcp/mcp_probe_server.py
)"
ARTIFACT_ROOT="$(
  readlink -m "${CODEX_MCP_E2E_ARTIFACT_ROOT:-temp/v2-regression/mcp}"
)"
mkdir -p "$ARTIFACT_ROOT"
CASE_DIR="$(mktemp -d "$ARTIFACT_ROOT/probe_codex_mcp-manual-XXXXXX")"
printf '%s\n' 'owned by the manual Codex MCP regression' \
  >"$CASE_DIR/.actrail-mcp-test-root"

RUNTIME_ROOT="$(
  readlink -m "${CODEX_MCP_E2E_MANUAL_RUNTIME_ROOT:-/tmp}"
)"
test "$RUNTIME_ROOT" != "/"
mkdir -p "$RUNTIME_ROOT"
RUNTIME_DIR="$(
  mktemp -d "$RUNTIME_ROOT/actrail-codex-mcp-manual-XXXXXX"
)"
printf '%s\n' 'owned by the manual Codex MCP runtime' \
  >"$RUNTIME_DIR/.actrail-mcp-runtime-root"
printf '%s\n' "$RUNTIME_DIR" >"$CASE_DIR/runtime-dir.txt"

TOKEN="$(
  "$REGRESSION_PYTHON" -c \
    'import secrets; print(secrets.token_hex(6))'
)"
LOCAL_SERVER="${CODEX_MCP_E2E_LOCAL_SERVER_NAME:-actrail_codex_stdio}"
TOOL_NAME="${CODEX_MCP_E2E_TOOL_NAME:-emit_marker}"
TOOL_DESCRIPTION_PADDING_BYTES="${CODEX_MCP_E2E_TOOL_DESCRIPTION_PADDING_BYTES:-8192}"
[[ "$LOCAL_SERVER" =~ ^[A-Za-z][A-Za-z0-9_-]*$ ]]
[[ "$TOOL_NAME" =~ ^[A-Za-z][A-Za-z0-9_-]*$ ]]
[[ "$TOOL_DESCRIPTION_PADDING_BYTES" =~ ^[0-9]+$ ]]
test "$TOOL_DESCRIPTION_PADDING_BYTES" -gt 4095

LOCAL_TOOL_ID="mcp__${LOCAL_SERVER}__${TOOL_NAME}"
LOCAL_MARKER="CODEX_STDIO_${TOKEN}"
FINAL_MARKER="CODEX_MCP_FINAL_${TOKEN}"
LOCAL_EVENTS="$CASE_DIR/${LOCAL_SERVER}.events.jsonl"
LAUNCH_LOG="$CASE_DIR/codex-launch.log"

MCP_ARGS="$(
  jq -cn \
    --arg script "$PROBE_SCRIPT" \
    --arg server "$LOCAL_SERVER" \
    --arg tool "$TOOL_NAME" \
    --arg marker "$LOCAL_MARKER" \
    --arg events "$LOCAL_EVENTS" \
    --arg padding "$TOOL_DESCRIPTION_PADDING_BYTES" \
    '[
       $script,
       "--server-name", $server,
       "--tool-name", $tool,
       "--marker", $marker,
       "--event-log", $events,
       "--tool-description-padding-bytes", $padding
     ]'
)"
PYTHON_TOML="$(jq -Rn --arg value "$PYTHON_BIN" '$value')"
SERVER_KEY="mcp_servers.${LOCAL_SERVER}"

OPERATOR_CONFIG="$RUNTIME_DIR/actraild.conf"
OPERATOR_PATCH="$RUNTIME_DIR/actraild.patch.toml"
TLS_SYNC_SOCKET="$RUNTIME_DIR/run/tls-sync.sock"
test "$(printf '%s' "$TLS_SYNC_SOCKET" | wc -c)" -lt 108

DAEMON=(sudo -E target/release/actraild --config "$OPERATOR_CONFIG")
CONTROL=(sudo -E target/release/actrailctl --config "$OPERATOR_CONFIG")
CODEX_CONTROL=(
  sudo -E
  "HOME=$CODEX_AUTH_USER_HOME"
  "CODEX_HOME=$CODEX_AUTH_CONFIG_HOME"
  target/release/actrailctl --config "$OPERATOR_CONFIG"
)
VIEWER=(sudo -E target/release/actrailviewer --config "$OPERATOR_CONFIG")
DAEMON_STARTED=0

cleanup_runtime() {
  local original_status=$?
  trap - EXIT
  set +e
  if test "$DAEMON_STARTED" -eq 1; then
    "${DAEMON[@]}" stop >/dev/null 2>&1
  fi
  printf 'manual artifacts retained at: %s\n' "$CASE_DIR"
  printf 'manual runtime retained at: %s\n' "$RUNTIME_DIR"
  return "$original_status"
}
trap cleanup_runtime EXIT

toml_string() {
  jq -Rn --arg value "$1" '$value'
}

{
  printf '[control]\n'
  printf 'socket_path = %s\n' \
    "$(toml_string "$RUNTIME_DIR/run/control.sock")"
  printf 'pid_file = %s\n' \
    "$(toml_string "$RUNTIME_DIR/run/actraild.pid")"
  printf 'log_path = %s\n' \
    "$(toml_string "$RUNTIME_DIR/log/actraild.log")"
  printf '\n[storage.sqlite]\n'
  printf 'path = %s\n' \
    "$(toml_string "$RUNTIME_DIR/data/actrail.sqlite")"
  printf '\n[storage.retention]\n'
  printf 'enabled = false\n'
  printf '\n[export.snapshot]\n'
  printf 'directory = %s\n' \
    "$(toml_string "$RUNTIME_DIR/data/export")"
  printf '\n[payload.tls]\n'
  printf 'sync_event_socket_path = %s\n' \
    "$(toml_string "$TLS_SYNC_SOCKET")"
  printf '\n[cluster.report]\n'
  printf 'spool_dir = %s\n' \
    "$(toml_string "$RUNTIME_DIR/data/cluster-spool")"
  printf 'state_path = %s\n' \
    "$(toml_string "$RUNTIME_DIR/data/cluster-report-state.sqlite")"
  printf '\n[cluster.center]\n'
  printf 'root_dir = %s\n' \
    "$(toml_string "$RUNTIME_DIR/data/cluster")"
  printf '\n[plugins.discovery]\n'
  printf 'directory = %s\n' \
    "$(toml_string "$RUNTIME_DIR/plugins")"
} >"$OPERATOR_PATCH"

printf 'case: %s\nruntime: %s\ntool: %s\nconfig: %s\n' \
  "$CASE_DIR" "$RUNTIME_DIR" "$LOCAL_TOOL_ID" "$OPERATOR_CONFIG"
```

#### 预期结果

`CASE_DIR` 是本轮独占证据目录。Codex MCP override 只定义一个本地 stdio
server。`RUNTIME_DIR` 是短路径运行时目录，`tls-sync.sock` 的字节长度小于 Linux
Unix socket 的 108-byte 上限。probe 参数与当前代码中的
`McpProbeWorkspace.server_arguments()` 一致：不含 `--transport`，并保留大于
4095 bytes 的 tool description padding。运行时 patch 与
`ActrailRuntime.isolated()` 隔离相同的状态，不会操作默认 control socket、PID、
日志或 SQLite。

### 步骤 3：初始化并启动隔离的 AcTrail daemon

#### 手动指令

```bash
"${DAEMON[@]}" init -f --patch "$OPERATOR_PATCH"
"${DAEMON[@]}" stop
"${CONTROL[@]}" clean
"${DAEMON[@]}" start
DAEMON_STARTED=1
test -S "$RUNTIME_DIR/run/control.sock"
```

#### 预期结果

四个生命周期命令均成功，daemon 监听本轮目录中的
`$RUNTIME_DIR/run/control.sock`，而不是 `/run/actrail/control.sock`。

### 步骤 4：通过单层 launch 运行真实 Codex MCP 调用

#### 手动指令

```bash
PROMPT="$(
  printf '%s' \
    "Use $LOCAL_TOOL_ID with {\"marker\":\"$LOCAL_MARKER\"}. " \
    "After the result returns, reply with \"$FINAL_MARKER\"."
)"

"${CODEX_CONTROL[@]}" launch -- \
  "$CODEX_BIN" exec \
  --ephemeral \
  -m "${CODEX_E2E_MODEL:-gpt-5.5}" \
  -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}" \
  -c "$SERVER_KEY.command=$PYTHON_TOML" \
  -c "$SERVER_KEY.args=$MCP_ARGS" \
  "$PROMPT" \
  </dev/null 2>&1 | tee "$LAUNCH_LOG"
```

#### 预期结果

launch 输出一个 `trace trace-N entered Active`，Codex 在 sudo 子进程中仍能读取
与预检相同的认证目录，只额外取得本轮配置的本地 stdio 工具，调用完成后输出
`$FINAL_MARKER`。命令不包含远程 MCP，也不包含初始化超时的旧等待或重试逻辑。

### 步骤 5：核验 Codex 输出、probe 执行证据和 trace id

#### 手动指令

```bash
rg -F -- "$FINAL_MARKER" "$LAUNCH_LOG" >/dev/null

jq -se \
  --arg server "$LOCAL_SERVER" \
  --arg tool "$TOOL_NAME" \
  --arg marker "$LOCAL_MARKER" '
    [.[] | select(.event == "tool_execution")] as $executions
    | [
        .[]
        | select(
            .event == "message"
            and .direction == "client_to_server"
            and .message.method == "tools/call"
          )
      ] as $requests
    | [
        .[]
        | select(
            .event == "message"
            and .direction == "server_to_client"
          )
        | .message.result.tools[]?
        | select(.name == $tool)
        | (.description | length)
      ] as $description_lengths
    | ($executions | length) == 1
      and ($requests | length) == 1
      and $executions[0].server == $server
      and $executions[0].tool == $tool
      and $executions[0].marker == $marker
      and $executions[0].arguments == {marker: $marker}
      and $requests[0].message.params
        == {name: $tool, arguments: {marker: $marker}}
      and $requests[0].message.id == $executions[0].request_id
      and (($description_lengths | max) > 4095)
      and (
        [
          .[]
          | select(
              .event == "message"
              and .direction == "server_to_client"
              and .message.id == $executions[0].request_id
              and .message.result.content
                == [{type: "text", text: $marker}]
              and .message.result.structuredContent == {marker: $marker}
              and .message.result.isError == false
            )
        ]
        | length
      ) == 1
  ' "$LOCAL_EVENTS" >/dev/null

TRACE_IDS="$(
  sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p' \
    "$LAUNCH_LOG"
)"
test "$(printf '%s\n' "$TRACE_IDS" | sed '/^$/d' | wc -l)" -eq 1
TRACE_ID="$(printf '%s\n' "$TRACE_IDS" | tail -n1)"
printf 'trace-%s\n' "$TRACE_ID"
```

#### 预期结果

final marker 存在；JSONL 中恰好有一次目标 `tools/call`、一次真实执行和一个
对应成功响应；tool description 超过 4095 bytes；launch 日志中只有一个 trace
id。

### 步骤 6：安全停止并核验固定 stdio MCP 语义图

#### 手动指令

```bash
"${DAEMON[@]}" stop
DAEMON_STARTED=0

TRACE_STATE="$(
  "${VIEWER[@]}" --output-format json traces |
    jq -r --argjson trace_id "$TRACE_ID" '
      .traces[]
      | select(.trace_id_raw == $trace_id)
      | "\(.state)/\(.health)"
    '
)"
test "$TRACE_STATE" = "Exited/Clean"

"${VIEWER[@]}" --output-format json actions \
  --trace-id "$TRACE_ID" >"$CASE_DIR/actions.json"

jq -e \
  --arg server "$LOCAL_SERVER" \
  --arg tool "$TOOL_NAME" '
    [.actions[] | select(.kind | startswith("mcp."))] as $mcp
    | [$mcp[] | select(.kind == "mcp.tool_call")] as $roots
    | [
        .links[]
        | select(
            (.role | startswith("mcp."))
            or .role == "command.contains_mcp_tool_call"
          )
      ] as $links
    | ($mcp | length) == 5
      and (([$mcp[] | .kind] | sort) == (
        [
          "mcp.tool_call",
          "mcp.request",
          "mcp.response",
          "mcp.stdout",
          "mcp.stdin"
        ] | sort
      ))
      and ($mcp | all(
        .status == "success"
        and .completeness == "complete"
        and .end_time_unix_nanos != null
        and .attributes["mcp.server.name"] == $server
        and .attributes["mcp.tool.name"] == $tool
        and .attributes["mcp.transport"] == "stdio"
      ))
      and ($roots | length) == 1
      and $roots[0].attributes["mcp.execution.status"] == "success"
      and $roots[0].attributes["mcp.tool.id"] == $tool
      and ($roots[0].attributes["llm.response.action_id"] | length) > 0
      and ($roots[0].attributes["llm.tool_call.id"] | length) > 0
      and $roots[0].attributes["llm.tool_call.name"]
        == ("mcp__" + $server + "__" + $tool)
      and ($links | length) == 5
      and (([$links[] | .role] | sort) == (
        [
          "command.contains_mcp_tool_call",
          "mcp.tool_call.request",
          "mcp.tool_call.response",
          "mcp.request.stdout",
          "mcp.response.stdin"
        ] | sort
      ))
      and ($links | all(
        .valid == true and .confidence == "observed"
      ))
  ' "$CASE_DIR/actions.json" >/dev/null

jq '{
    mcp_action_count: [
      .actions[] | select(.kind | startswith("mcp."))
    ] | length,
    roots: [
      .actions[]
      | select(.kind == "mcp.tool_call")
      | {
          id: .action_id,
          status,
          completeness,
          server: .attributes["mcp.server.name"],
          tool: .attributes["mcp.tool.name"],
          transport: .attributes["mcp.transport"],
          llm_response: .attributes["llm.response.action_id"],
          llm_tool_id: .attributes["llm.tool_call.id"],
          llm_tool_name: .attributes["llm.tool_call.name"]
        }
    ]
  }' "$CASE_DIR/actions.json"

"${VIEWER[@]}" --output-format json payloads \
  --trace-id "$TRACE_ID" >"$CASE_DIR/payloads.json"
jq -e '
  ([.payloads[] | select(.protocol_hint == "stdout")] | length) == 0
  and ([.payloads[] | select(.protocol_hint == "stdin")] | length) > 0
' "$CASE_DIR/payloads.json" >/dev/null

"${VIEWER[@]}" diagnostics --trace-id "$TRACE_ID" \
  >"$CASE_DIR/diagnostics.txt"
rg -F \
  'MCP stdio observation mcp_stdio_candidate_stream_discarded: reason=candidate_truncated' \
  "$CASE_DIR/diagnostics.txt" >/dev/null
```

#### 预期结果

trace 为 `Exited/Clean`；恰好有五个 `success/complete` 的 stdio MCP action、
一个 root 和五条唯一角色 link；root 的 LLM attribution 字段非空且工具名精确
匹配。默认配置下没有 stdout payload，但至少有一个 MCP stdin payload；诊断中
存在预期的可恢复 `candidate_truncated` 记录。自动测例还会逐项检查 action 引用
和所有 payload evidence 是否指向已持久化 segment。

### 步骤 7：核验并清理手动产物

#### 手动指令

```bash
test "$DAEMON_STARTED" -eq 0
test "$(dirname "$CASE_DIR")" = "$ARTIFACT_ROOT"
test -f "$CASE_DIR/.actrail-mcp-test-root"
case "$(basename "$CASE_DIR")" in
  probe_codex_mcp-manual-*) ;;
  *) false ;;
esac
test "$(dirname "$RUNTIME_DIR")" = "$RUNTIME_ROOT"
test -f "$RUNTIME_DIR/.actrail-mcp-runtime-root"
case "$(basename "$RUNTIME_DIR")" in
  actrail-codex-mcp-manual-*) ;;
  *) false ;;
esac
sudo rm -rf -- "$CASE_DIR" "$RUNTIME_DIR"
rmdir "$ARTIFACT_ROOT" 2>/dev/null || true
trap - EXIT
```

#### 预期结果

Codex、stdio MCP server 和隔离 daemon 均已退出。证据目录与短路径运行时目录
分别通过父目录、目录名前缀和 ownership marker 检查后才会删除；中途失败时
trap 会停止隔离 daemon，并同时保留 `$CASE_DIR` 和 `$RUNTIME_DIR` 供排查。

## 配置

| 环境变量 | 默认值 | 作用 |
| --- | --- | --- |
| `ACTRAIL_TEST_PYTHON` | `python3` | Python 3.10+ 解释器；手动流程也用它启动 probe server |
| `CODEX_E2E_BINARY` | 自动查找 `codex` | Codex 可执行文件 |
| `CODEX_E2E_MODEL` | `gpt-5.5` | Codex model |
| `CODEX_E2E_REASONING_EFFORT` | `low` | reasoning effort |
| `CODEX_MCP_E2E_COMMAND_TIMEOUT_SECONDS` | `30` | AcTrail 管理及停止超时 |
| `CODEX_MCP_E2E_LAUNCH_TIMEOUT_SECONDS` | `180` | Codex workload 超时 |
| `CODEX_MCP_E2E_ARTIFACT_ROOT` | `temp/v2-regression/mcp` | 隔离证据目录 |
| `CODEX_MCP_E2E_MANUAL_RUNTIME_ROOT` | `/tmp` | 手动流程的短路径运行时父目录；用于容纳 Unix socket、日志和 SQLite |
| `CODEX_MCP_E2E_LOCAL_SERVER_NAME` | `actrail_codex_stdio` | MCP server 名称 |
| `CODEX_MCP_E2E_TOOL_NAME` | `emit_marker` | MCP tool 名称 |
| `CODEX_MCP_E2E_TOOL_DESCRIPTION_PADDING_BYTES` | `8192` | 填充 tool description，使 `tools/list` 响应越过 4095-byte stdio capture ABI 上限；必须大于 4095 |

server 和 tool 名称必须匹配 `[A-Za-z][A-Za-z0-9_-]*`。非法配置直接失败。
