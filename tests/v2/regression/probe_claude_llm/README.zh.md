# Probe Claude LLM

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/probe_claude_llm/run_e2e.py
```

脚本从 `CLAUDE_E2E_BINARY` 或当前环境查找 Claude，先执行外部可用性
检查，再通过单层 `actrailctl launch` 捕获一次无会话、无工具的 Claude 请求。

# 步骤摘要

1. 检查 AcTrail release binaries 和 Claude 外部可用性。
2. 执行 `actraild init -f → actraild stop → actrailctl clean → actraild start`。
3. 生成随机 marker，并通过 `actrailctl launch` 请求 Claude 原样回答。
4. 验证 Claude 标准输出包含 marker，并提取唯一 trace id。
5. 安全停止 daemon，等待 trace 和 post-trace 数据排空。
6. 验证 trace 为 `Exited/Clean`。
7. 验证终态 `llm.call/request/response` 完整配对，response 包含 marker，
   request 具有 canonical content 证据。

# 手动测试

本测例只涉及本地 CLI、daemon 和 viewer，没有需要浏览器操作的页面；Bash
命令就是最直接的手动复现方式。以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
CLAUDE_BIN="${CLAUDE_E2E_BINARY:-$(command -v claude)}"
test -n "$CLAUDE_BIN"
"$CLAUDE_BIN" \
  -p 'Reply with exactly "CLAUDE_PREFLIGHT_OK" and nothing else.' \
  --model "${CLAUDE_E2E_MODEL:-sonnet}" \
  --no-session-persistence \
  --safe-mode \
  --permission-mode dontAsk \
  --tools ""
```

### 预期结果

三个 AcTrail 二进制均存在且可执行；Claude 命令退出状态为成功，标准输出包含
`CLAUDE_PREFLIGHT_OK`。Claude 缺失、未登录、model/provider 不可用或网络不可达
属于外部条件不满足，自动测试会标记为 `SKIPPED`。

## 步骤2：初始化并启动 AcTrail

### 手动指令

```bash
sudo -E target/release/actraild init -f
sudo -E target/release/actraild stop
sudo -E target/release/actrailctl clean
sudo -E target/release/actraild start
```

### 预期结果

四条命令均成功；最后一条输出 daemon pid 和
`/run/actrail/control.sock`。AcTrail 二进制缺失或 daemon 无法启动应判为
`FAILED`，不是 `SKIPPED`。

## 步骤3：生成 marker 并运行 Claude

### 手动指令

```bash
CASE_MARKER="A$(python3 -c 'import secrets; print(secrets.token_hex(5))')"
LAUNCH_OUTPUT="$(
  sudo -E target/release/actrailctl launch -- \
    "$CLAUDE_BIN" \
    "Reply with exactly \"$CASE_MARKER\" and nothing else. Do not use tools." \
    --print \
    --output-format text \
    --model "${CLAUDE_E2E_MODEL:-sonnet}" \
    --no-session-persistence \
    --safe-mode \
    --permission-mode dontAsk \
    --tools "" 2>&1
)"
printf '%s\n' "$LAUNCH_OUTPUT"
```

### 预期结果

输出中出现 `trace trace-N entered Active`，Claude 随后只回答
`$CASE_MARKER`，命令最终成功退出。该命令不得创建可恢复的 Claude session。

## 步骤4：验证回答并提取 trace id

### 手动指令

```bash
printf '%s\n' "$LAUNCH_OUTPUT" | rg -F "$CASE_MARKER"
TRACE_ID="$(
  printf '%s\n' "$LAUNCH_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
test "$(printf '%s\n' "$TRACE_ID" | sed '/^$/d' | wc -l)" -eq 1
printf 'trace-%s\n' "$TRACE_ID"
```

### 预期结果

marker 查找成功，只提取到一个数字 trace id。

## 步骤5：安全停止 daemon

### 手动指令

```bash
sudo -E target/release/actraild stop
```

### 预期结果

命令成功；daemon 在退出前完成 trace 收尾、post-trace 任务和告警写入排空。

## 步骤6：验证 trace 最终状态

### 手动指令

```bash
sudo -E target/release/actrailviewer --output-format json traces |
  jq --argjson trace_id "$TRACE_ID" '
    .traces[]
    | select(.trace_id_raw == $trace_id)
    | {trace_id, state, health}
  '
```

### 预期结果

只显示目标 trace，且 `state` 为 `Exited`、`health` 为 `Clean`。

## 步骤7：验证 LLM action 与 marker

### 手动指令

```bash
sudo -E target/release/actrailviewer --output-format json actions \
  --trace-id "$TRACE_ID" > /tmp/actrail-claude-actions.json

jq --arg marker "$CASE_MARKER" '
  [.actions[] | select(.kind == "llm.call")] as $calls
  | [.actions[] | select(.kind == "llm.request")] as $requests
  | [.actions[] | select(.kind == "llm.response")] as $responses
  | {
      calls: ($calls | length),
      requests: ($requests | length),
      responses: ($responses | length),
      nonterminal: [
        .actions[]
        | select(
            (.kind == "llm.call"
             or .kind == "llm.request"
             or .kind == "llm.response"
             or .kind == "sse.stream")
            and .status == "in_progress"
          )
        | .action_id
      ],
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
' /tmp/actrail-claude-actions.json
```

### 预期结果

`calls`、`requests`、`responses` 均大于零且数量相同；`nonterminal` 为空；
`call_links` 中每个 call 恰好有一个 request link 和一个 response link，且没有
request/response 被重复使用；`canonical_requests` 和 `marker_responses`
均非空。
