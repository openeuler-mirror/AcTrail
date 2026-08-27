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
5. 停止 daemon，等待事件排空和 trace finalization 完成。
6. 从数据库验证 trace 为 `Exited/Clean`，并验证 LLM actions 完整配对，
   response 包含 marker，request 具有 canonical content 证据。

# 手动测试

本测例没有浏览器交互面，使用 Bash 直接复现。以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
cargo build --release \
  -p tls_probe_point_finder \
  --bin tls-probe-point-finder
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
test -n "$XIAOO_BIN"
target/release/tls-probe-point-finder fast \
  --provider rustls \
  --source auto \
  "$XIAOO_BIN"
"$XIAOO_BIN" \
  --cli run \
  --no-tools \
  --max-turns 1 \
  --prompt 'Reply with exactly "XIAOO_PREFLIGHT_OK" and nothing else.'
```

### 预期结果

AcTrail binaries 均可执行；使用 HTTPS/rustls 的 xiaoO 必须先输出包含
`rustls_buffer_plaintext` 和 `rustls_take_received_plaintext` 的完整 probe plan，随后
成功输出 `XIAOO_PREFLIGHT_OK`。保留符号的 Rust v0 ELF 可以通过符号解析；stripped
构建必须匹配一套已验证的静态特征。xiaoO 缺失、provider/model 配置不可用、认证失效
或网络不可达时，自动测试标记为 `SKIPPED`；xiaoO 能回答但 finder 无完整 plan 时，
HTTPS 内容仍不可见，最终 LLM action 校验会失败。plain HTTP provider route 不需要
rustls plan，可直接通过 socket plaintext 捕获。

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

## 步骤5：停止 daemon 并完成 finalization

### 手动指令

```bash
sudo -E target/release/actraild stop
```

### 预期结果

daemon 在已捕获事件排空、trace finalization 和数据持久化完成后成功退出。

## 步骤6：验证最终 trace、LLM action 与 marker

### 手动指令

```bash
TRACE_STATE="$(
  sudo -E target/release/actrailviewer --output-format json traces |
    jq -r --argjson trace_id "$TRACE_ID" '
      .traces[]
      | select(.trace_id_raw == $trace_id)
      | "\(.state)/\(.health)"
    '
)"
test "$TRACE_STATE" = "Exited/Clean"

sudo -E target/release/actrailviewer --output-format json actions \
  --trace-id "$TRACE_ID" > /tmp/actrail-xiaoo-actions.json

jq --arg marker "$CASE_MARKER" '
  [.actions[] | select(.kind == "llm.call")] as $calls
  | [.actions[] | select(.kind == "llm.request")] as $requests
  | [.actions[] | select(.kind == "llm.response")] as $responses
  | [.actions[]
      | select(.kind == "http.message"
               and .attributes["http.operation"] == "request")] as $http_requests
  | [.actions[]
      | select(.kind == "http.message"
               and .attributes["http.operation"] == "response"
               and (((.attributes.status_code // "0") | tonumber) >= 400))]
      as $failed_http_responses
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
      ],
      failed_http_requests: [
        $failed_http_responses[] as $response
        | $http_requests[]
        | select(.action_id == $response.attributes["http.request.action_id"])
        | {
            stream_key: .attributes.stream_key,
            method: .attributes.method,
            target: .attributes.target,
            status_code: $response.attributes.status_code
          }
      ]
    }
' /tmp/actrail-xiaoo-actions.json
```

### 预期结果

trace 为 `Exited/Clean`；call 与 request 计数相等且每个 call 恰好有一个
request link。已有的 LLM response 必须各自拥有唯一 response link，不能复用。
缺少 LLM response 的 call 只能是 trace-close 后的 terminal partial/error，并且其
request 的 stream、method 和 path 必须与 `failed_http_requests` 中尚未使用的
HTTP 4xx/5xx request 一致。`canonical_requests` 和 `marker_responses` 均非空。
