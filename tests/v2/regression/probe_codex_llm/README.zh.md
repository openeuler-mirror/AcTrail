# Probe Codex LLM

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/probe_codex_llm/run_e2e.py
```

脚本从 `CODEX_E2E_BINARY` 或当前环境查找 Codex，先执行外部可用性检查，
再通过双层 `actrailctl launch` 验证嵌套 trace 中的 Codex LLM 采集。

# 步骤摘要

1. 检查 AcTrail release binaries 和 Codex 外部可用性。
2. 初始化、清理并启动 AcTrail。
3. 生成随机 marker，通过外层 launch 再启动内层 launch 和 `codex exec`。
4. 验证 Codex 标准输出包含 marker，并从两个 trace id 中选择内层 trace。
5. 验证内层 trace 为 `Exited/Clean`。
6. 验证 `llm.call/request/response` 数量一致、关系完整，response 包含 marker，
   request 具有 canonical content 证据。

# 手动测试

本测例没有浏览器交互面，使用 Bash 直接复现。以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
CODEX_BIN="${CODEX_E2E_BINARY:-$(command -v codex)}"
test -n "$CODEX_BIN"
"$CODEX_BIN" exec \
  --ephemeral \
  -m "${CODEX_E2E_MODEL:-gpt-5.5}" \
  -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}" \
  'Reply with exactly "CODEX_PREFLIGHT_OK" and nothing else. Do not use tools.'
```

### 预期结果

AcTrail binaries 均可执行；Codex 命令成功并输出 `CODEX_PREFLIGHT_OK`，且
`--ephemeral` 不保存 session。Codex 缺失、未认证、model/provider 不可用或
网络不可达时，自动测试标记为 `SKIPPED`。

## 步骤2：初始化并启动 AcTrail

### 手动指令

```bash
sudo -E target/release/actraild init -f
sudo -E target/release/actraild stop
sudo -E target/release/actrailctl clean
sudo -E target/release/actraild start
```

### 预期结果

所有命令成功，daemon 正在监听 `/run/actrail/control.sock`。AcTrail 自身缺失
或启动失败属于 `FAILED`。

## 步骤3：生成 marker 并运行嵌套 launch

### 手动指令

```bash
CASE_MARKER="A$(python3 -c 'import secrets; print(secrets.token_hex(5))')"
LAUNCH_OUTPUT="$(
  sudo -E target/release/actrailctl launch -- \
    target/release/actrailctl launch -- \
      "$CODEX_BIN" exec \
      --ephemeral \
      -m "${CODEX_E2E_MODEL:-gpt-5.5}" \
      -c "model_reasoning_effort=${CODEX_E2E_REASONING_EFFORT:-low}" \
      "Reply with exactly \"$CASE_MARKER\" and nothing else. Do not use tools." \
      2>&1
)"
printf '%s\n' "$LAUNCH_OUTPUT"
```

### 预期结果

输出中先后出现两个不同的 `trace trace-N entered Active`；Codex 输出
`$CASE_MARKER` 并成功退出。

## 步骤4：验证回答并选择内层 trace

### 手动指令

```bash
printf '%s\n' "$LAUNCH_OUTPUT" | rg -F "$CASE_MARKER"
TRACE_IDS="$(
  printf '%s\n' "$LAUNCH_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
test "$(printf '%s\n' "$TRACE_IDS" | sed '/^$/d' | wc -l)" -eq 2
test "$(printf '%s\n' "$TRACE_IDS" | sort -u | wc -l)" -eq 2
INNER_TRACE_ID="$(printf '%s\n' "$TRACE_IDS" | tail -n1)"
printf 'inner trace-%s\n' "$INNER_TRACE_ID"
```

### 预期结果

marker 查找成功；恰好得到两个不同 trace id，第二个 id 是直接包裹 Codex 的
内层 trace。

## 步骤5：验证内层 trace 状态

### 手动指令

```bash
for TRACE_ATTEMPT in $(seq 1 30); do
  TRACE_STATE="$(
    sudo -E target/release/actrailviewer --output-format json traces |
      jq -r --argjson trace_id "$INNER_TRACE_ID" '
        .traces[]
        | select(.trace_id_raw == $trace_id)
        | "\(.state)/\(.health)"
      '
  )"
  test "$TRACE_STATE" = "Exited/Clean" && break
  sleep 1
done
test "$TRACE_STATE" = "Exited/Clean"
printf '%s\n' "$TRACE_STATE"
```

### 预期结果

内层 trace 的 `state` 为 `Exited`，`health` 为 `Clean`。

## 步骤6：验证 LLM action 与 marker

### 手动指令

```bash
for ACTION_ATTEMPT in $(seq 1 30); do
  sudo -E target/release/actrailviewer --output-format json actions \
    --trace-id "$INNER_TRACE_ID" > /tmp/actrail-codex-actions.json
  ACTION_COUNT="$(
    jq '[.actions[] | select(.kind == "llm.response")] | length' \
      /tmp/actrail-codex-actions.json
  )"
  test "$ACTION_COUNT" -gt 0 && break
  sleep 1
done
test "$ACTION_COUNT" -gt 0

jq --arg marker "$CASE_MARKER" '
  [.actions[] | select(.kind == "llm.call")] as $calls
  | [.actions[] | select(.kind == "llm.request")] as $requests
  | [.actions[] | select(.kind == "llm.response")] as $responses
  | {
      calls: ($calls | length),
      requests: ($requests | length),
      responses: ($responses | length),
      call_links: [
        .links[]
        | select(
            .valid == true
            and (.role == "llm.call.request" or .role == "llm.call.response")
          )
        | {
            call: .parent_action_id,
            role,
            child: .child_action_id
          }
      ],
      canonical_requests: [
        $requests[]
        | select(
            .attributes["llm.request.content_state"] == "canonical_blocks"
            and ((.attributes["llm.request.canonical_body_hash"] // "")
                 | startswith("sha256:"))
            and (((.attributes["llm.request.canonical_body_bytes"] // "0")
                  | tonumber) > 0)
          )
        | .action_id
      ],
      marker_responses: [
        $responses[]
        | select(
            ((.attributes["llm.response.content_text"] // "") | contains($marker))
            or ((.attributes["llm.response.output_text"] // "") | contains($marker))
          )
        | .action_id
      ]
    }
' /tmp/actrail-codex-actions.json
```

### 预期结果

三个计数均大于零且相等；`call_links` 中每个 call 恰好有一个 request link
和一个 response link，且没有 request/response 被重复使用；
`canonical_requests`、`marker_responses` 均非空。

## 步骤7：停止 daemon

### 手动指令

```bash
sudo -E target/release/actraild stop
```

### 预期结果

daemon 成功停止，没有遗留运行中的测试进程。
