# otel-jsonl 动作筛选端到端测试

验证用户通过 Web API 加载 `otel-jsonl` 插件并修改动作种类勾选配置后，插件只导出被勾选的 action kind。

本用例使用 `curl` 复现 Web 前端请求，并以检测阶段选出的真实 Agent 调用同时产生
具有代表性的进程、文件、命令调用和 LLM 行为。核心覆盖集合至少包括 `process.exec`、
`file.read`、`command.invocation`、`llm.call`、`llm.request` 和
`llm.response`，但不穷举所有 action kind。

当前 Agent 调用表示为带有 `invocation.kind=agent` 属性的
`command.invocation`；本用例按 action kind 筛选，不把该属性当成独立 kind。

`file.tty_io` 由 recording 层在上游过滤，不属于 exporter 的可配置 action kind；
插件配置和 Schema 均不得暴露该项。

本用例验证 Web API 与实际 OTEL JSONL，不验证浏览器 DOM 或视觉样式。

# Quick Run

默认执行并清理：

```bash
sudo -E python3 tests/v2/regression/plugins/otel-jsonl/run_e2e.py --cleanup
```

单独调试并保留运行状态和 case workspace：

```bash
sudo -E python3 tests/v2/regression/plugins/otel-jsonl/run_e2e.py --no-cleanup
```

通过聚合入口执行时，清理策略由 `test_all.py` 的外层参数统一控制：

```bash
sudo -E python3 tests/v2/regression/test_all.py \
  --case plugin_otel_jsonl \
  --cleanup
```

runner 根据 `--work-root` 为 definition 注入独立 `work_dir`。默认目录为
`/tmp/actrail-regression/plugin_otel_jsonl`；测例本身不解析或决定该路径。
启用 `--cleanup` 时，runner 还会删除该 case 的 runner log；使用
`--no-cleanup` 才会保留 workspace 和日志用于调试。默认日志目录统一位于
`/tmp/actrail-regression/logs`。

自动化用例从刷新后的默认模板生成 case-local `actraild.conf`，再用最小 patch 将
`plugins.discovery.directory` 指向仓库中的 `examples/plugins/builtin`。因此测试使用
当前源码配套的官方 `otel-jsonl` manifest、配置和 Schema，不读取或覆盖用户
`~/.actrail/plugins` 下可能被编辑过的安装资产。设置
`OTEL_JSONL_E2E_OPERATOR_CONFIG` 时可以显式覆盖 case-local config 路径。

# 步骤摘要

1. 清理历史状态并启动 `actraild`。
2. 启动 `actrailweb`。
3. 通过 `curl` 加载插件并提交动作勾选配置。
4. 启动一次真实 Agent 调用，观测采集。
5. 从 OTEL JSONL 中验证只存在被勾选的 action kind。
6. 使用不同勾选配置循环执行步骤 3、4、5。
7. 清理插件、Web 和守护进程。

循环矩阵：

| 轮次 | 勾选的 action kind | 期望导出 |
| --- | --- | --- |
| A | `process.exec`、`file.read`、`command.invocation` | 只有进程、文件和命令调用三类代表动作 |
| B | `llm.call`、`llm.request`、`llm.response` | 只有完整 LLM 三元组 |
| C | 从 A 随机一项、B 随机一项，再从剩余四项随机一项 | 只导出本次随机选中的三种动作 |
| D | 上述六种 action kind | 同时导出进程、文件、命令调用和 LLM 代表动作 |

轮次 A 和 B 证明不同类别可以独立勾选；轮次 C 每次覆盖一个不同的跨组组合，
并保证不会退化成 A 或 B；轮次 D 证明 Web 配置能够组合全部代表类别。这四轮只
覆盖有代表性的主路径，不要求为每一个 boolean 组合建立分支。

# 手动测试

以下命令均从仓库根目录执行。

## 步骤 1：清理并启动 actraild

### 手动指令

```bash
set -euo pipefail

test "$(uname -s)" = "Linux"
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
test -x target/release/actrailweb
command -v curl
command -v jq
command -v timeout
sudo -n true

REGRESSION_TMP_ROOT=/tmp/actrail-regression
CASE_DIR=$REGRESSION_TMP_ROOT/plugin_otel_jsonl
test "$REGRESSION_TMP_ROOT" = /tmp/actrail-regression
test "$CASE_DIR" = /tmp/actrail-regression/plugin_otel_jsonl
sudo rm -rf -- "$CASE_DIR"
mkdir -p "$CASE_DIR"

OPERATOR_CONFIG=/etc/actrail/actraild.conf
WEB_URL=http://127.0.0.1:18080
PLUGIN_PACKAGE=otel-jsonl
PLUGIN_INSTANCE=v2.otel-jsonl
WEB_PID=
DAEMON_STARTED=0
WEB_STARTED=0
PLUGIN_LOADED=0
AGENT_KIND=
AGENT_BIN=
AGENT_COMMAND=()
```

定义统一清理函数，并立即注册 `EXIT` trap。它会恢复插件原始配置、卸载插件、
停止 Web 和 daemon、清理 trace，最后删除本用例目录：

```bash
cleanup() {
  local original_status=${1:-0}
  local cleanup_failed=0

  trap - EXIT
  set +e

  if test "$PLUGIN_LOADED" -eq 1 \
    && test -s "$CASE_DIR/config-initial.json"; then
    jq '{config: .config}' \
      "$CASE_DIR/config-initial.json" \
      >"$CASE_DIR/config-restore-request.json" \
      || cleanup_failed=1
    curl -fsS \
      -X POST \
      -H 'Content-Type: application/json' \
      --data-binary "@$CASE_DIR/config-restore-request.json" \
      "$WEB_URL/api/plugins/runtime/config?instance_id=$PLUGIN_INSTANCE" \
      >/dev/null 2>&1 \
      || cleanup_failed=1
  fi

  if test "$PLUGIN_LOADED" -eq 1; then
    curl -fsS \
      -X POST \
      "$WEB_URL/api/plugins/runtime/unload?instance_id=$PLUGIN_INSTANCE" \
      >/dev/null 2>&1 \
      || cleanup_failed=1
  fi

  if test "$WEB_STARTED" -eq 1 && test -n "${WEB_PID:-}"; then
    sudo kill "$WEB_PID" >/dev/null 2>&1 \
      || cleanup_failed=1
    wait "$WEB_PID" 2>/dev/null
  fi

  if test "$DAEMON_STARTED" -eq 1; then
    sudo -E target/release/actraild stop >/dev/null 2>&1 \
      || cleanup_failed=1
    sudo -E target/release/actrailctl clean >/dev/null 2>&1 \
      || cleanup_failed=1
  fi

  test "$CASE_DIR" = /tmp/actrail-regression/plugin_otel_jsonl \
    && sudo rm -rf -- "$CASE_DIR" \
    || cleanup_failed=1
  sudo rmdir /tmp/actrail-regression 2>/dev/null

  if test "$original_status" -eq 0 && test "$cleanup_failed" -ne 0; then
    return 1
  fi
  return "$original_status"
}

trap 'cleanup $?' EXIT
```

定义 Agent binary 到非交互参数的唯一映射：

```bash
build_agent_command() {
  local kind=$1
  local binary=$2
  local prompt=$3

  case "$kind" in
    xiaoo)
      AGENT_COMMAND=(
        "$binary"
        --cli run
        --no-tools
        --max-turns 1
        --prompt "$prompt"
      )
      ;;
    pi)
      AGENT_COMMAND=(
        "$binary"
        -p "$prompt"
        --no-session
      )
      ;;
    opencode)
      AGENT_COMMAND=(
        "$binary"
        run
        "$prompt"
      )
      ;;
    claude)
      AGENT_COMMAND=(
        "$binary"
        "$prompt"
        --print
        --output-format text
        --model "${CLAUDE_E2E_MODEL:-sonnet}"
        --no-session-persistence
        --safe-mode
        --permission-mode dontAsk
        --tools ""
      )
      ;;
    codex)
      AGENT_COMMAND=(
        "$binary"
        exec
        --ephemeral
        -m "${CODEX_E2E_MODEL:-gpt-5.5}"
        -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}"
        "$prompt"
      )
      ;;
    *)
      return 1
      ;;
  esac
}
```

严格按照 `xiaoo → pi → opencode → claude → codex` 探测。探测不仅检查文件存在，
还执行一次最小请求；只有命令成功且输出 marker 才选择该 Agent：

```bash
for candidate_kind in xiaoo pi opencode claude codex; do
  case "$candidate_kind" in
    xiaoo)
      candidate_bin="${XIAOO_E2E_BINARY:-$(command -v xiaoo || true)}"
      ;;
    pi)
      candidate_bin="${PI_E2E_BINARY:-$(command -v pi || true)}"
      ;;
    opencode)
      candidate_bin="${OPENCODE_E2E_BINARY:-$(command -v opencode || true)}"
      ;;
    claude)
      candidate_bin="${CLAUDE_E2E_BINARY:-$(command -v claude || true)}"
      ;;
    codex)
      candidate_bin="${CODEX_E2E_BINARY:-$(command -v codex || true)}"
      ;;
  esac

  test -n "$candidate_bin" && test -x "$candidate_bin" || continue

  preflight_marker="ACTRAIL_${candidate_kind^^}_PREFLIGHT_OK"
  build_agent_command \
    "$candidate_kind" \
    "$candidate_bin" \
    "Reply with exactly \"$preflight_marker\" and nothing else. Do not use tools."

  if timeout 120 "${AGENT_COMMAND[@]}" \
      >"$CASE_DIR/preflight-$candidate_kind.log" 2>&1 \
      && grep -F "$preflight_marker" \
        "$CASE_DIR/preflight-$candidate_kind.log" >/dev/null; then
    AGENT_KIND=$candidate_kind
    AGENT_BIN=$candidate_bin
    break
  fi
done

if test -z "$AGENT_KIND"; then
  printf '%s\n' \
    'SKIPPED: no usable agent binary in xiaoo/pi/opencode/claude/codex'
  exit 0
fi

test -x "$AGENT_BIN"
printf 'selected_agent=%s binary=%s\n' "$AGENT_KIND" "$AGENT_BIN"
```

选择完成后再正常清理并启动 `actraild`：

```bash
sudo -E target/release/actraild init -f
sudo -E target/release/actraild stop || true
sudo -E target/release/actrailctl clean
sudo -E target/release/actraild start
DAEMON_STARTED=1
```

等待守护进程就绪：

```bash
for _ in $(seq 1 100); do
  if sudo -E target/release/actrailviewer \
    --config "$OPERATOR_CONFIG" \
    --output-format json \
    traces >"$CASE_DIR/initial-traces.json" 2>"$CASE_DIR/actrailviewer.err"; then
    break
  fi
  sleep 0.1
done

jq -e '.traces | type == "array" and length == 0' \
  "$CASE_DIR/initial-traces.json"
```

### 预期结果

- 严格按 `xiaoo`、`pi`、`opencode`、`claude`、`codex` 顺序选择第一个通过最小
  请求的 Agent。
- `AGENT_KIND` 和 `AGENT_BIN` 唯一确定，后续采集复用同一个 Agent。
- `actraild` 启动成功。
- 最迟 10 秒内 `actrailviewer traces` 返回空的 trace 数组。

五个候选 Agent 全部缺失，或全部因认证、model/provider、网络等外部条件无法完成
最小请求时，本用例记为 `SKIPPED`。选出 Agent 后，AcTrail 自身的启动、采集或
导出异常均记为 `FAILED`。

## 步骤 2：启动 Web

### 手动指令

```bash
sudo -E target/release/actrailweb \
  --config "$OPERATOR_CONFIG" \
  --addr 127.0.0.1 \
  --port 18080 \
  >"$CASE_DIR/actrailweb.log" 2>&1 &
WEB_PID=$!
WEB_STARTED=1

for _ in $(seq 1 100); do
  if curl -fsS "$WEB_URL/api/plugins/catalog" \
    >"$CASE_DIR/catalog.json"; then
    break
  fi
  sleep 0.1
done

jq -e '
  .available == true
  and .runtime_available == true
  and any(.packages[];
    .package_key == "otel-jsonl"
    and .plugin_id == "otel-jsonl"
    and .runtime == "builtin"
    and .activation_ready == true
  )
' "$CASE_DIR/catalog.json"
```

### 预期结果

- `actrailweb` 在 `127.0.0.1:18080` 启动。
- 最迟 10 秒内 Web API 可访问。
- 插件目录中存在可激活的 builtin `otel-jsonl`。

## 步骤 3：通过 curl 加载插件并确认勾选协议

### 手动指令

```bash
curl -fsS \
  -X POST \
  -H 'Content-Type: application/json' \
  --data '{"instance_id":"v2.otel-jsonl"}' \
  "$WEB_URL/api/plugins/catalog/load?package=$PLUGIN_PACKAGE" \
  >"$CASE_DIR/load.json"

jq -e '
  .available == true
  and .plugin.instance_id == "v2.otel-jsonl"
  and .plugin.plugin_id == "otel-jsonl"
  and .plugin.runtime == "builtin"
  and .plugin.state == "active"
' "$CASE_DIR/load.json"
PLUGIN_LOADED=1

curl -fsS \
  "$WEB_URL/api/plugins/runtime/config?instance_id=$PLUGIN_INSTANCE" \
  >"$CASE_DIR/config-initial.json"

jq -e '
  .available == true
  and .editable == true
  and (.config.action_kinds.default | type) == "boolean"
  and (.config.action_kinds | has("file.tty_io") | not)
  and .schema.properties.action_kinds.type == "object"
  and .schema.properties.action_kinds.additionalProperties == false
  and (
    .schema.properties.action_kinds.properties
    | has("file.tty_io")
    | not
  )
  and .schema.properties.action_kinds.properties["llm.call"].type == "boolean"
  and .schema.properties.action_kinds.properties["llm.request"].type == "boolean"
  and .schema.properties.action_kinds.properties["llm.response"].type == "boolean"
  and .schema.properties.action_kinds.properties["process.exec"].type == "boolean"
  and .schema.properties.action_kinds.properties["file.read"].type == "boolean"
  and .schema.properties.action_kinds.properties["command.invocation"].type == "boolean"
' "$CASE_DIR/config-initial.json"
```

### 预期结果

- 插件实例为 `v2.otel-jsonl`，状态为 `active`。
- 配置可编辑。
- `default`、进程、文件、命令调用和 LLM 代表动作均由 Schema 声明为 boolean，
  Web 前端可以将其呈现为复选框。
- `action_kinds` 禁止额外属性，配置与 Schema 均不包含 `file.tty_io`。

## 步骤 4：定义一轮配置、采集和验证

### 手动指令

以下函数先取消所有 action kind，再模拟用户勾选本轮需要的代表项：

```bash
apply_action_filter() {
  local llm_call=$1
  local llm_request=$2
  local llm_response=$3
  local process_exec=$4
  local file_read=$5
  local command_invocation=$6

  curl -fsS \
    "$WEB_URL/api/plugins/runtime/config?instance_id=$PLUGIN_INSTANCE" \
    >"$CASE_DIR/config-current.json"

  jq \
    --argjson llm_call "$llm_call" \
    --argjson llm_request "$llm_request" \
    --argjson llm_response "$llm_response" \
    --argjson process_exec "$process_exec" \
    --argjson file_read "$file_read" \
    --argjson command_invocation "$command_invocation" \
    --arg otel_path "$CASE_DIR/otel.jsonl" \
    '
      .config
      | .path = $otel_path
      | .overwrite_enabled = true
      | .action_kinds |= with_entries(.value = false)
      | .action_kinds.default = false
      | .action_kinds["llm.call"] = $llm_call
      | .action_kinds["llm.request"] = $llm_request
      | .action_kinds["llm.response"] = $llm_response
      | .action_kinds["process.exec"] = $process_exec
      | .action_kinds["file.read"] = $file_read
      | .action_kinds["command.invocation"] = $command_invocation
    ' "$CASE_DIR/config-current.json" \
    >"$CASE_DIR/config-candidate.json"

  jq -n \
    --slurpfile config "$CASE_DIR/config-candidate.json" \
    '{config: $config[0]}' \
    >"$CASE_DIR/config-request.json"

  curl -fsS \
    -X POST \
    -H 'Content-Type: application/json' \
    --data-binary "@$CASE_DIR/config-request.json" \
    "$WEB_URL/api/plugins/runtime/config/validate?instance_id=$PLUGIN_INSTANCE" \
    >"$CASE_DIR/config-validation.json"
  jq -e '.valid == true' "$CASE_DIR/config-validation.json"

  curl -fsS \
    -X POST \
    -H 'Content-Type: application/json' \
    --data-binary "@$CASE_DIR/config-request.json" \
    "$WEB_URL/api/plugins/runtime/config?instance_id=$PLUGIN_INSTANCE" \
    >"$CASE_DIR/config-updated.json"

  OTEL_PATH=$(
    jq -er '.config.path | select(type == "string" and length > 0)' \
      "$CASE_DIR/config-updated.json"
  )
  test "$OTEL_PATH" = "$CASE_DIR/otel.jsonl"
}
```

以下函数从指定 trace 的 OTEL resource 中提取所有已导出的 action kind：

```bash
extract_marker_kinds() {
  local marker=$1

  sudo -E jq -s \
    --arg marker "$marker" \
    '
      def string_attr($attributes; $key):
        first(
          $attributes[]?
          | select(.key == $key)
          | .value.stringValue
        );

      [
        .[]
        | .resourceSpans[]?
        | select(
            string_attr(
              .resource.attributes;
              "actrail.trace.display_name"
            ) == $marker
          )
        | .scopeSpans[]?.spans[]?
        | string_attr(.attributes; "actrail.action.kind")
      ]
      | sort
      | unique
    ' "$OTEL_PATH"
}
```

单轮流程如下：

```bash
run_round() {
  local round=$1
  local llm_call=$2
  local llm_request=$3
  local llm_response=$4
  local process_exec=$5
  local file_read=$6
  local command_invocation=$7
  local expected=$8
  local marker
  local trace_id
  local trace_state=
  local actual='[]'

  apply_action_filter \
    "$llm_call" \
    "$llm_request" \
    "$llm_response" \
    "$process_exec" \
    "$file_read" \
    "$command_invocation"

  marker="OTEL_JSONL_${round}_$(date +%s%N)"

  build_agent_command \
    "$AGENT_KIND" \
    "$AGENT_BIN" \
    "Reply with exactly \"$marker\" and nothing else. Do not use tools."

  sudo -E target/release/actrailctl \
    --config "$OPERATOR_CONFIG" \
    launch \
    --name "$marker" \
    -- bash -lc 'cat /etc/hostname >/dev/null; exec "$@"' \
    actrail-otel-jsonl \
    "${AGENT_COMMAND[@]}" \
    >"$CASE_DIR/launch-$round.log" 2>&1

  grep -F "$marker" "$CASE_DIR/launch-$round.log"

  trace_id=$(
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p' \
      "$CASE_DIR/launch-$round.log" \
    | head -n1
  )
  test -n "$trace_id"

  for _ in $(seq 1 300); do
    trace_state=$(
      sudo -E target/release/actrailviewer \
        --config "$OPERATOR_CONFIG" \
        --output-format json \
        traces \
      | jq -r \
          --argjson trace_id "$trace_id" \
          '
            .traces[]
            | select(.trace_id_raw == $trace_id)
            | "\(.state)/\(.health)"
          '
    )
    test "$trace_state" = "Exited/Clean" && break
    sleep 0.1
  done
  test "$trace_state" = "Exited/Clean"

  for _ in $(seq 1 300); do
    if sudo test -s "$OTEL_PATH"; then
      actual=$(extract_marker_kinds "$marker")
      if jq -e \
        --argjson expected "$expected" \
        '. == $expected' <<<"$actual" >/dev/null; then
        break
      fi
    fi
    sleep 0.1
  done

  jq -n \
    --arg round "$round" \
    --arg marker "$marker" \
    --argjson trace_id "$trace_id" \
    --argjson expected "$expected" \
    --argjson actual "$actual" \
    '{
      round: $round,
      marker: $marker,
      trace_id: $trace_id,
      expected: $expected,
      actual: $actual,
      passed: ($actual == $expected)
    }' | tee "$CASE_DIR/result-$round.json"

  jq -e '.passed == true' "$CASE_DIR/result-$round.json"
}
```

### 预期结果

- 配置提交前通过服务端校验。
- 每轮配置先把 `default` 和所有已知 action kind 设为 `false`，再设置本轮勾选项。
- 每轮都由检测阶段选中的同一个 Agent 发起真实 LLM 请求并产生唯一 trace。
- trace 最迟 30 秒进入 `Exited/Clean`。
- OTEL 结果最迟 30 秒与本轮勾选集合完全一致；多出或缺少任何 kind 均为
  `FAILED`。

## 步骤 5：循环四轮并验证

### 手动指令

第一轮勾选进程、文件和命令调用代表动作：

```bash
run_round \
  execution-context \
  false \
  false \
  false \
  true \
  true \
  true \
  '["command.invocation","file.read","process.exec"]'
```

第二轮勾选完整 LLM action 组：

```bash
run_round \
  llm-complete \
  true \
  true \
  true \
  false \
  false \
  false \
  '["llm.call","llm.request","llm.response"]'
```

第三轮从 A、B 各随机选择一项，再从剩余四项中随机选择第三项：

```bash
EXECUTION_CONTEXT_KINDS=(
  process.exec
  file.read
  command.invocation
)
LLM_COMPLETE_KINDS=(
  llm.call
  llm.request
  llm.response
)

RANDOM_EXECUTION_KIND=$(
  printf '%s\n' "${EXECUTION_CONTEXT_KINDS[@]}" | shuf -n 1
)
RANDOM_LLM_KIND=$(
  printf '%s\n' "${LLM_COMPLETE_KINDS[@]}" | shuf -n 1
)
RANDOM_THIRD_KIND=$(
  printf '%s\n' \
    "${EXECUTION_CONTEXT_KINDS[@]}" \
    "${LLM_COMPLETE_KINDS[@]}" \
  | grep -vxF \
      -e "$RANDOM_EXECUTION_KIND" \
      -e "$RANDOM_LLM_KIND" \
  | shuf -n 1
)
RANDOM_THREE=$(
  printf '%s\n' \
    "$RANDOM_EXECUTION_KIND" \
    "$RANDOM_LLM_KIND" \
    "$RANDOM_THIRD_KIND" \
  | jq -R . \
  | jq -sc 'sort'
)

kind_flag() {
  if jq -e --arg kind "$1" 'index($kind) != null' \
    <<<"$RANDOM_THREE" >/dev/null; then
    printf 'true'
  else
    printf 'false'
  fi
}

run_round \
  mixed-random-three \
  "$(kind_flag llm.call)" \
  "$(kind_flag llm.request)" \
  "$(kind_flag llm.response)" \
  "$(kind_flag process.exec)" \
  "$(kind_flag file.read)" \
  "$(kind_flag command.invocation)" \
  "$RANDOM_THREE"
```

第四轮合并勾选六种代表动作：

```bash
run_round \
  representative-combined \
  true \
  true \
  true \
  true \
  true \
  true \
  '["command.invocation","file.read","llm.call","llm.request","llm.response","process.exec"]'
```

汇总：

```bash
jq -s \
  'map({
    round,
    marker,
    trace_id,
    expected,
    actual,
    passed
  })' \
  "$CASE_DIR"/result-*.json \
  | tee "$CASE_DIR/results.json"

jq -e 'length == 4 and all(.[]; .passed == true)' \
  "$CASE_DIR/results.json"
```

### 预期结果

- `execution-context` 只导出 `command.invocation`、`file.read` 和 `process.exec`，
  不导出实际调用中产生的 LLM action。
- `llm-complete` 完整导出 `llm.call`、`llm.request`、`llm.response`。
- `mixed-random-three` 只导出本次随机选中的三种 action kind，其中至少一项来自
  execution-context，一项来自 llm-complete。
- `representative-combined` 同时导出六种代表动作。
- 四轮均为 `passed: true`。

## 步骤 6：验证插件状态并清理

### 手动指令

```bash
curl -fsS "$WEB_URL/api/plugins/runtime" \
  >"$CASE_DIR/runtime-final.json"

jq -e '
  .available == true
  and any(.plugins[];
    .instance_id == "v2.otel-jsonl"
    and .state == "active"
    and .dropped_records == 0
  )
' "$CASE_DIR/runtime-final.json"

cleanup_status=0
cleanup 0 || cleanup_status=$?
set -e
trap - EXIT

test "$cleanup_status" -eq 0
test ! -e /tmp/actrail-regression/plugin_otel_jsonl
```

失败或 `SKIPPED` 场景由 `EXIT` trap 执行同一套清理，不保留本用例产生的日志、
JSONL、trace 或临时配置。`/tmp/actrail-regression` 仅在为空时删除，不影响并行
或其他回归用例。Python 自动化实现中，task 不执行这段清理；case cleanup hook
负责恢复外部状态，runner 负责最终删除 workspace。

### 预期结果

- 插件在四轮结束后仍为 `active`，且 `dropped_records == 0`。
- 插件原始配置已恢复。
- 插件、Web、守护进程和测试 trace 均被清理。
- `/tmp/actrail-regression/plugin_otel_jsonl` 不存在，没有保留测试证据。
