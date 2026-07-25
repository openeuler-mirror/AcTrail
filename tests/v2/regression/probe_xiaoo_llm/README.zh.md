# Probe xiaoO LLM

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/probe_xiaoo_llm/run_e2e.py
```

脚本从 `XIAOO_E2E_BINARY` 或当前环境查找 xiaoO，先执行外部可用性检查，
再通过单层 `actrailctl launch` 捕获一次无工具、单轮请求。

# 步骤摘要

1. 检查 AcTrail release binaries 和 xiaoO 外部可用性。
2. 初始化、清理并启动 AcTrail。
3. 生成随机 marker，以 `--no-tools --max-turns 1` 请求 xiaoO 原样回答。
4. 验证标准输出包含 marker，并提取唯一 trace id。
5. 等待 trace 达到 `Exited/Clean`。
6. 验证 LLM actions 完整配对，response 包含 marker，request 具有 canonical
   content 证据。
7. 停止 daemon。

# 手动测试

本测例没有浏览器交互面，使用 Bash 直接复现。以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
test -n "$XIAOO_BIN"
"$XIAOO_BIN" \
  --cli run \
  --no-tools \
  --max-turns 1 \
  --prompt 'Reply with exactly "XIAOO_PREFLIGHT_OK" and nothing else.'
```

### 预期结果

AcTrail binaries 均可执行；xiaoO 成功输出 `XIAOO_PREFLIGHT_OK`。xiaoO
缺失、provider/model 配置不可用、认证失效或网络不可达时，自动测试标记为
`SKIPPED`。

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

## 步骤3：生成 marker 并运行 xiaoO

### 手动指令

```bash
CASE_MARKER="A$(python3 -c 'import secrets; print(secrets.token_hex(5))')"
LAUNCH_OUTPUT="$(
  sudo -E target/release/actrailctl launch -- \
    "$XIAOO_BIN" \
    --cli run \
    --no-tools \
    --max-turns 1 \
    --prompt "Reply with exactly \"$CASE_MARKER\" and nothing else. Do not use tools." \
    2>&1
)"
printf '%s\n' "$LAUNCH_OUTPUT"
```

### 预期结果

输出中出现 `trace trace-N entered Active`，xiaoO 输出 `$CASE_MARKER` 并在
一轮内成功退出。

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

## 步骤5：等待并验证 trace 状态

### 手动指令

```bash
for TRACE_ATTEMPT in $(seq 1 30); do
  TRACE_STATE="$(
    sudo -E target/release/actrailviewer --output-format json traces |
      jq -r --argjson trace_id "$TRACE_ID" '
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

最多等待约 30 秒后显示 `Exited/Clean`。始终缺失、未退出或 health 不为
`Clean` 都属于测试失败。

## 步骤6：验证 LLM action 与 marker

### 手动指令

```bash
for ACTION_ATTEMPT in $(seq 1 30); do
  sudo -E target/release/actrailviewer --output-format json actions \
    --trace-id "$TRACE_ID" > /tmp/actrail-xiaoo-actions.json
  ACTION_COUNT="$(
    jq '[.actions[] | select(.kind == "llm.response")] | length' \
      /tmp/actrail-xiaoo-actions.json
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
' /tmp/actrail-xiaoo-actions.json
```

### 预期结果

三个 action 计数均大于零且相等；`call_links` 中每个 call 恰好有一个
request link 和一个 response link，且没有 request/response 被重复使用；
`canonical_requests` 和 `marker_responses` 均非空。

## 步骤7：停止 daemon

### 手动指令

```bash
sudo -E target/release/actraild stop
```

### 预期结果

daemon 成功停止，没有遗留运行中的测试进程。
