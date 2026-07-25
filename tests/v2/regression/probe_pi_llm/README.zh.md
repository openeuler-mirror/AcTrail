# Probe pi LLM

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/probe_pi_llm/run_e2e.py
```

脚本从当前环境的 `PATH` 解析默认 `pi`，先执行外部可用性检查，再通过
单层 `actrailctl launch` 执行：

```bash
pi -p "prompt" --no-session
```

# 步骤摘要

1. 检查 AcTrail release binaries 和默认 Pi agent 的外部可用性。
2. 初始化、清理并启动 AcTrail。
3. 生成随机 marker，以 `--no-session` 模式请求 Pi 原样回答。
4. 验证标准输出包含 marker，并提取唯一 trace id。
5. 安全停止 daemon，验证 trace 为 `Exited/Clean`。
6. 验证终态 LLM actions 完整配对，response 包含 marker，request 具有
   canonical content 证据。

# 手动测试

本测例没有浏览器交互面，使用 Bash 直接复现。以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
PI_BIN="$(command -v pi)"
test -n "$PI_BIN"
"$PI_BIN" -p 'Reply with exactly "PI_PREFLIGHT_OK" and nothing else.' --no-session
```

### 预期结果

AcTrail binaries 均可执行；环境默认 Pi agent 成功输出 `PI_PREFLIGHT_OK`，且
不保存 session。Pi 缺失、未配置 provider/model、认证失效或网络不可达时，
自动测试标记为 `SKIPPED`。

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

## 步骤3：生成 marker 并运行 Pi

### 手动指令

```bash
CASE_MARKER="A$(python3 -c 'import secrets; print(secrets.token_hex(5))')"
LAUNCH_OUTPUT="$(
  sudo -E target/release/actrailctl launch -- \
    "$PI_BIN" \
    -p "Reply with exactly \"$CASE_MARKER\" and nothing else. Do not use tools." \
    --no-session 2>&1
)"
printf '%s\n' "$LAUNCH_OUTPUT"
```

### 预期结果

输出中出现 `trace trace-N entered Active`，Pi 输出 `$CASE_MARKER` 并成功退出；
运行结束后没有新增可恢复 session。

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

## 步骤5：安全停止并验证 trace

### 手动指令

```bash
sudo -E target/release/actraild stop
sudo -E target/release/actrailviewer --output-format json traces |
  jq --argjson trace_id "$TRACE_ID" '
    .traces[]
    | select(.trace_id_raw == $trace_id)
    | {trace_id, state, health}
  '
```

### 预期结果

daemon 成功停止；目标 trace 的 `state` 为 `Exited`、`health` 为 `Clean`。

## 步骤6：验证 LLM action 与 marker

### 手动指令

```bash
sudo -E target/release/actrailviewer --output-format json actions \
  --trace-id "$TRACE_ID" > /tmp/actrail-pi-actions.json

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
' /tmp/actrail-pi-actions.json
```

### 预期结果

三个 action 计数均大于零且相等；`nonterminal` 为空；`call_links` 中每个
call 恰好有一个 request link 和一个 response link，且没有 request/response
被重复使用；`canonical_requests` 和 `marker_responses` 均非空。
