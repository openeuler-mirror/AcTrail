# OTEL exporter 选择与动作筛选回归

## 测试目标

本测试验证 builtin `otel-jsonl` 插件可以在同一个运行实例中选择文件或
JSON-RPC 2.0 over HTTP(S) exporter，并且只导出 Web 配置中启用的 action
kind。

手动测试以 Web 页面为主要插件操作入口，覆盖插件发现、加载、配置、状态检查和
卸载；每个 Web 操作都给出等价的 `curl`，便于定位页面请求或在无浏览器环境中执行
相同操作。daemon 启停、真实 Agent 执行、接收端启动和结果检查本来就是本地操作，
直接使用 CLI。

测试要确认以下结果：

- `Exporter` 选择 `File` 时只显示文件配置，并把 OTLP JSON 写入指定 JSONL 文件；
- `Exporter` 选择 `JSON-RPC 2.0 over HTTP(S)` 时只显示 JRPC 配置，并向指定
  endpoint 发送 JSON-RPC 请求；
- 两种 exporter 都只输出已启用的 action kind；
- JRPC 遇到一次 HTTP 503 和一次响应超时后，以相同 request ID 重试并最终成功；
- exporter 的网络等待不阻塞上游，测试结束时插件保持 `Active`、
  `dropped_records=0` 且没有运行错误。

Web 的 `Update configuration` 只更新当前运行实例，不会改写插件包中的
`otel-jsonl.config.toml`。本测试不验证 semantic action 的终态、identity、exit 或
调用配对规则，这些由其他 `tests/v2` 回归负责。

## 手动测试前提

以下操作会重新安装 release、执行 `actrailctl clean`，并使用主机上的 AcTrail
运行目录。只能在允许清理现有测试数据的开发机上执行，不要与生产实例共用。

需要：

- Rust/Cargo release 构建环境；
- `curl`、`jq`、`rg`；
- 已登录且可调用的真实 Agent。下面以 `xiaoo` 为例；
- 能以 root 启动 AcTrail。

命令默认从仓库根目录执行。每段 `curl` 都是对应 Web 操作的等价方式；修改状态的
Web 操作和 `curl` 二选一，不要对同一个实例重复执行。

## 步骤 1：构建并安装最新 release

### CLI 操作

```bash
cd /home/yzh/projects/AcTrail
bash scripts/install-release.sh
```

### 预期现象

release 构建成功，`target/release/` 下存在 `actraild`、`actrailctl`、
`actrailviewer` 和 `actrailweb`，安装脚本最后打印二进制与官方插件的安装位置。

## 步骤 2：初始化、清理并启动 daemon

### CLI 操作

```bash
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual
BIN="$REPO/target/release"

mkdir -p "$WORK"
cat > "$WORK/actraild.patch.toml" <<EOF
[plugins.discovery]
directory = "$REPO/examples/plugins/builtin"
EOF

sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" \
  init -f --patch "$WORK/actraild.patch.toml"
sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" stop
sudo -E "$BIN/actrailctl" --config "$WORK/actraild.conf" clean
sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" start
```

### 预期现象

四条 AcTrail 命令均成功。最后一条打印 daemon pid 和 control socket；测试配置中的
插件发现目录指向当前仓库的 `examples/plugins/builtin`。

## 步骤 3：启动 Web 并打开 Plugins 页面

### CLI 操作

在第二个终端执行：

```bash
cd /home/yzh/projects/AcTrail
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual

sudo -E "$REPO/target/release/actrailweb" \
  --config "$WORK/actraild.conf" \
  --addr 127.0.0.1 \
  --port 18080
```

### 预期现象

终端打印 `actrailweb listening on http://127.0.0.1:18080`。浏览器打开
<http://127.0.0.1:18080>，进入 `Plugins` 页面后可以看到
`Installed Plugins` 和 `Plugin candidates`。

## 步骤 4：通过 Web 发现并加载 otel-jsonl

### Web 操作

1. 在 `Plugin candidates` 标题右侧点击 `Refresh`；
2. 展开 `otel-jsonl`，确认 `Runtime` 为 `Available`、候选状态为 `Unloaded`；
3. 点击 `Load plugin`；
4. 在 `Runtime instance name` 输入 `manual.otel-jsonl`；
5. 再点击弹窗中的 `Load plugin`。

### 等价 curl

```bash
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS "$WEB_BASE/api/plugins/catalog" |
  jq '.packages[]
      | select(.package_key == "otel-jsonl")
      | {package_key, plugin_id, runtime, activation_ready, issue}'

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data '{"instance_id":"manual.otel-jsonl"}' \
  "$WEB_BASE/api/plugins/catalog/load?package=otel-jsonl" |
  jq '.plugin | {instance_id, plugin_id, runtime, state}'
```

### 预期现象

候选从 `Plugin candidates` 移到 `Loaded plugin instances`。实例 ID 为
`manual.otel-jsonl`，状态显示 `Active`，runtime 显示 `builtin`。

## 步骤 5：通过 Web 选择 File exporter

### Web 操作

1. 在 `Loaded plugin instances` 展开 `manual.otel-jsonl`；
2. 点击 `Configuration`；
3. 在 `Exporter` 下拉框选择 `File`，确认页面只显示 `File exporter` 配置；
4. 保持 `Queue capacity` 为 `1024`；
5. 设置 `Output path` 为
   `/tmp/actrail-otel-jsonl-manual/file.otlp.jsonl`；
6. 打开 `Overwrite existing file`，将 `Flush every spans` 设为 `1`；
7. 在 `action_kinds` 中关闭 `Default` 和其他 action，仅打开
   `process.exec`、`file.read`、`command.invocation`；
8. 点击 `Test configuration`；通过后点击 `Update configuration`。

### 等价 curl

打开 `Configuration` 对应读取当前完整配置：

```bash
WORK=/tmp/actrail-otel-jsonl-manual
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  > "$WORK/config-document.json"
```

生成与页面输入相同的完整配置：

```bash
jq --arg path "$WORK/file.otlp.jsonl" '
  {
    config: (
      .config
      | .exporter = "file"
      | .queue_capacity = 1024
      | .file.path = $path
      | .file.overwrite_enabled = true
      | .file.flush_every_spans = 1
      | .action_kinds = (.action_kinds | with_entries(.value = false))
      | .action_kinds."process.exec" = true
      | .action_kinds."file.read" = true
      | .action_kinds."command.invocation" = true
    )
  }
' "$WORK/config-document.json" > "$WORK/file-config-request.json"
```

`Test configuration` 和 `Update configuration` 分别对应：

```bash
curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/file-config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config/validate?instance_id=$INSTANCE" |
  jq

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/file-config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" |
  jq '.config | {exporter, queue_capacity, file, action_kinds}'
```

### 预期现象

测试配置后页面显示 `Test passed — ready to update`；更新后显示
`Runtime configuration updated.`。重新展开配置时 `Exporter` 仍为 `File`，且
JSON-RPC 配置不显示。validate API 返回 `"valid": true`。

## 步骤 6：启动真实 Agent 并检查文件导出

### CLI 操作

回到第一个终端执行：

```bash
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual
BIN="$REPO/target/release"
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
test -x "$XIAOO_BIN"

FILE_MARKER="OTEL_FILE_MANUAL_$(date +%s%N)"
FILE_LAUNCH_OUTPUT="$(
  sudo -E env HOME="$HOME" PATH="$PATH" \
    "$BIN/actrailctl" --config "$WORK/actraild.conf" \
    launch --name "$FILE_MARKER" -- \
    bash -lc 'cat /etc/hostname >/dev/null; exec "$@"' \
    actrail-otel-jsonl-manual \
    "$XIAOO_BIN" --cli run --no-tools --max-turns 1 \
    --prompt "Reply with exactly \"$FILE_MARKER\" and nothing else. Do not use tools." \
    2>&1
)"
printf '%s\n' "$FILE_LAUNCH_OUTPUT"

FILE_TRACE_ID="$(
  printf '%s\n' "$FILE_LAUNCH_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
test -n "$FILE_TRACE_ID"
```

查看上游实际产生的 action kinds：

```bash
sudo -E "$BIN/actrailviewer" --config "$WORK/actraild.conf" \
  --output-format json actions --trace-id "$FILE_TRACE_ID" |
  jq -r '.actions[].kind' |
  sort -u
```

等待导出文件写入并检查 exporter 输出：

```bash
for _ in $(seq 1 100); do
  test -s "$WORK/file.otlp.jsonl" && break
  sleep 0.1
done
test -s "$WORK/file.otlp.jsonl"

jq -r '
  ..
  | objects
  | select(.key? == "actrail.action.kind")
  | .value.stringValue
' "$WORK/file.otlp.jsonl" |
  sort -u
```

### 预期现象

Agent 输出包含唯一的 `trace trace-N entered Active` 和 `$FILE_MARKER`。viewer 中能
看到真实 Agent 产生的多类 action；文件导出结果只包含：

```text
command.invocation
file.read
process.exec
```

## 步骤 7：通过 CLI 启动 JRPC 接收端

### CLI 操作

在第三个终端执行：

```bash
cd /home/yzh/projects/AcTrail
WORK=/tmp/actrail-otel-jsonl-manual

python3 tests/v2/regression/otel_jsonl_action_filter/receiver.py \
  --output "$WORK/rpc.otlp.jsonl" \
  --fail-next 1 \
  --delay-next-ms 750
```

`--fail-next 1` 让接收端第一次请求返回 HTTP 503；`--delay-next-ms 750` 让下一次
请求等待 750ms 后再响应。这两个参数只用于手动验证重试；只验证正常导出时可以省略。

### 预期现象

接收端启动后打印：

```text
endpoint=http://127.0.0.1:<动态端口>/rpc
output=/tmp/actrail-otel-jsonl-manual/rpc.otlp.jsonl
```

保持该终端运行，并复制实际打印的 endpoint。

## 步骤 8：通过 Web 切换到 JSON-RPC exporter

### Web 操作

1. 回到 `manual.otel-jsonl` 的 `Configuration`；
2. 在 `Exporter` 选择 `JSON-RPC 2.0 over HTTP(S)`，确认页面只显示
   `JSON-RPC HTTP exporter` 配置；
3. `Endpoint` 填写步骤 7 打印的 endpoint；
4. `Method` 填写 `otel.export`；
5. `Connect timeout (ms)` 填写 `250`；
6. `Request timeout (ms)` 填写 `500`；
7. `Response body limit` 填写 `65536`；
8. `Maximum attempts` 填写 `3`；
9. `Retry backoff (ms)` 填写 `10`；
10. 在 `action_kinds` 中关闭 `Default` 和其他 action，仅打开
    `llm.call`、`llm.request`、`llm.response`；
11. 点击 `Test configuration`；通过后点击 `Update configuration`。

### 等价 curl

把接收端实际打印的地址写入 `RPC_ENDPOINT`：

```bash
WORK=/tmp/actrail-otel-jsonl-manual
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl
RPC_ENDPOINT='http://127.0.0.1:<动态端口>/rpc'

curl -fsS \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  > "$WORK/config-document.json"

jq --arg endpoint "$RPC_ENDPOINT" '
  {
    config: (
      .config
      | .exporter = "json_rpc_http"
      | .queue_capacity = 1024
      | .json_rpc_http.endpoint = $endpoint
      | .json_rpc_http.method = "otel.export"
      | .json_rpc_http.connect_timeout_ms = 250
      | .json_rpc_http.request_timeout_ms = 500
      | .json_rpc_http.response_body_max_bytes = 65536
      | .json_rpc_http.max_attempts = 3
      | .json_rpc_http.retry_backoff_ms = 10
      | .action_kinds = (.action_kinds | with_entries(.value = false))
      | .action_kinds."llm.call" = true
      | .action_kinds."llm.request" = true
      | .action_kinds."llm.response" = true
    )
  }
' "$WORK/config-document.json" > "$WORK/jrpc-config-request.json"

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/jrpc-config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config/validate?instance_id=$INSTANCE" |
  jq

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/jrpc-config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" |
  jq '.config | {exporter, queue_capacity, json_rpc_http, action_kinds}'
```

### 预期现象

配置验证通过并显示 `Runtime configuration updated.`。重新展开配置时
`Exporter` 仍为 `JSON-RPC 2.0 over HTTP(S)`，文件配置不显示。

## 步骤 9：再次启动真实 Agent 并检查 JRPC 请求

### CLI 操作

```bash
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual
BIN="$REPO/target/release"
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"

JRPC_MARKER="OTEL_JRPC_MANUAL_$(date +%s%N)"
JRPC_LAUNCH_OUTPUT="$(
  sudo -E env HOME="$HOME" PATH="$PATH" \
    "$BIN/actrailctl" --config "$WORK/actraild.conf" \
    launch --name "$JRPC_MARKER" -- \
    bash -lc 'cat /etc/hostname >/dev/null; exec "$@"' \
    actrail-otel-jsonl-manual \
    "$XIAOO_BIN" --cli run --no-tools --max-turns 1 \
    --prompt "Reply with exactly \"$JRPC_MARKER\" and nothing else. Do not use tools." \
    2>&1
)"
printf '%s\n' "$JRPC_LAUNCH_OUTPUT"

for _ in $(seq 1 100); do
  test -s "$WORK/rpc.otlp.jsonl" && break
  sleep 0.1
done
test -s "$WORK/rpc.otlp.jsonl"

jq -r '
  ..
  | objects
  | select(.key? == "actrail.action.kind")
  | .value.stringValue
' "$WORK/rpc.otlp.jsonl" |
  sort -u
```

### 预期现象

receiver 终端首先连续打印三次相同的 request ID：第一次请求收到 503，第二次请求
因 750ms 响应延迟超过 500ms 请求超时而重试，第三次请求成功。随后会继续打印新的
request ID 和累计 `received_documents`。

`rpc.otlp.jsonl` 中的 action kind 只包含：

```text
llm.call
llm.request
llm.response
```

第二次请求可能已经被接收端处理，但响应到达 exporter 前超时，因此输出中允许出现
相同 request ID 对应的重复 OTLP 文档。

## 步骤 10：通过 Web 检查插件状态

### Web 操作

1. 在 `Plugin candidates` 标题右侧点击 `Refresh`；
2. 在 `Loaded plugin instances` 展开 `manual.otel-jsonl`；
3. 检查状态为 `Active`；
4. 检查 `Records` 显示 `0 dropped`；
5. 等待 `Queue` 排空，检查 `Last error` 为 `none`、`Warnings` 为 `none`。

### 等价 curl

```bash
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS "$WEB_BASE/api/plugins/runtime" |
  jq --arg instance "$INSTANCE" '
    .plugins[]
    | select(.instance_id == $instance)
    | {
        instance_id,
        state,
        observed_records,
        dropped_records,
        queue_depth,
        queue_capacity,
        last_error,
        warnings
      }
  '
```

### 预期现象

实例保持 `active`，`observed_records` 大于零，`dropped_records` 为 `0`；队列最终为
`0/1024`，`last_error` 为空，warnings 为空。

## 步骤 11：通过 Web 卸载并清理环境

### Web 操作

1. 在 `Loaded plugin instances` 找到 `manual.otel-jsonl`；
2. 点击 `Unload`；
3. 在确认弹窗中点击 `Unload instance`。

### 等价 curl

```bash
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS -X POST \
  "$WEB_BASE/api/plugins/runtime/unload?instance_id=$INSTANCE" |
  jq
```

### CLI 清理

1. 在 receiver 终端按 `Ctrl-C`；
2. 在 `actrailweb` 终端按 `Ctrl-C`；
3. 回到第一个终端执行：

```bash
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual
BIN="$REPO/target/release"

sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" stop
sudo -E "$BIN/actrailctl" --config "$WORK/actraild.conf" clean
sudo rm -r -- /tmp/actrail-otel-jsonl-manual
```

### 预期现象

Web 中运行实例消失，`otel-jsonl` 重新出现在候选列表；receiver、Web 和 daemon
全部停止，手动测试产生的临时文件被删除。

## 自动执行

自动回归使用相同的 release、daemon、Web HTTP API、receiver 和真实 Agent 路径：

```bash
sudo -E env \
  PATH="$PATH" \
  CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
  RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
  python3 tests/v2/regression/otel_jsonl_action_filter/run_e2e.py --cleanup
```

自动回归通过 Web HTTP API 切换配置，不会操作浏览器 DOM；步骤 4、5、8、10、11
用于人工确认页面上的实际选择、条件字段、状态和生命周期操作。自动回归完成时应显示
两轮 file、两轮 JRPC 均通过，插件保持 active 且 `dropped_records=0`。
