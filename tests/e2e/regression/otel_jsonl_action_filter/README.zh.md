# OTEL exporter 选择与动作筛选回归

## 测试目标

验证 AcTrail 能将真实 Agent 的活动按所选动作类型导出到文件或网络接收端（内置 `otel-jsonl` 插件的 File、JSON-RPC exporter）。启用的类型有导出记录，禁用的类型没有导出记录。网络接收失败后，重试保留同一次投递的请求标识（request ID）。

## 场景设计

每轮直接启动真实 Agent，让它使用只读搜索工具查找测试目录中文件的指定内容，并自然总结结果。
搜索工具实际执行搜索命令并读取文件，生成 `process.exec`、`command.invocation`、`file.read`
及 LLM 请求与响应。准备阶段登记所选 Agent 的已知程序路径，使启动前完成探针准备。
Agent 工具权限请求为只读，模型轮数默认最多 4 轮，由各 CLI 适配器按支持能力配置。

先核实本轮已采到执行、文件读取与模型交互，再核对导出结果。对同一插件实例依次执行四轮配置切换（Web HTTP API），每轮记录独立的 trace ID。

| 轮次 | 导出位置 | 启用的动作类型 |
| --- | --- | --- |
| 执行活动 | 文件 | 程序执行、命令和文件读取（`process.exec`、`command.invocation`、`file.read`） |
| 模型交互 | 网络接收端 | 调用、请求和响应（`llm.call`、`llm.request`、`llm.response`） |
| 混合选择 | 文件 | 从上述六类中选三类，至少包含一类执行活动和一类模型交互 |
| 全部六类 | 网络接收端 | 上述六类动作 |

在模型交互轮次，让本地接收端先返回一次服务暂不可用（HTTP 503），再延迟一次响应直至发送端超时，最后恢复正常接收。以接收端实际收到的请求核对重试次数与请求标识。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限和可工作的 eBPF 环境 | 启动采集与治理 |
| AcTrail | release 产物和测例使用的官方插件包 | 执行、保存和查询测试结果 |
| Agent | 已登录且可调用的真实 Agent，如 XiaoO | 产生导出动作 |
| 接收端 | 文件输出目录与本地 JSON-RPC receiver | 核对文件和网络导出 |
| 操作工具 | curl、jq、rg | 管理插件并查询结果 |

## 测试步骤摘要

1. 刷新默认配置并启动 AcTrail、Web 和本地接收端。
2. 发现并加载插件，通过 Web 选择 File exporter 和动作类型。
3. 准备文本文件，直接运行真实 Agent 搜索文件内容。核实源动作中已有执行、命令、文件读取与 LLM 记录，再检查 JSONL 文件中的动作类型及导出内容。
4. 切换为 JSON-RPC exporter，运行真实 Agent 并记录接收请求。
5. 让接收端分别返回一次 HTTP 503 和一次超时，核对重试与请求标识。
6. 检查插件状态、丢弃计数和错误，再卸载插件并清理环境。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 导出位置 | 配置已更新，并产生本轮真实活动 | 记录出现在所选文件或网络接收端（File、JSON-RPC） |
| 动作筛选 | 本轮源记录已包含六类受检动作 | 导出记录的类型集合等于本轮启用的类型集合（action kind） |
| 网络重试 | 接收端发生一次 503 和一次超时 | 使用相同 request ID 重试并最终成功 |
| 运行状态 | 导出和重试完成 | 上游持续运行，插件 Active、dropped_records=0 且无运行错误 |

## 快速运行

自动回归使用相同的 release、daemon、Web HTTP API、receiver 和真实 Agent 路径：

```bash
sudo -E env \
  PATH="$PATH" \
  CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
  RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
  python3 tests/e2e/regression/otel_jsonl_action_filter/run_e2e.py --cleanup
```

自动回归通过管理接口切换配置（Web HTTP API）。完成时应显示两轮文件导出、两轮网络导出均通过，插件保持运行状态（Active），丢弃计数为零（`dropped_records=0`）。

轮数上限可通过 `OTEL_JSONL_ACTION_FILTER_E2E_AGENT_MAX_TURNS` 设置，默认值为 `4`。

## 手动测试

以下通过 Web 页面操作同一场景，并给出相应管理接口命令（`curl`）。选择导出方式后，页面应显示相应的文件或网络配置字段。页面字段显示与插件发现、加载、卸载由本节手动检查。

### 操作前提

以下操作会重新安装 release、执行 `actrailctl clean`，并使用主机上的 AcTrail
运行目录。只能在允许清理现有测试数据的开发机上执行，不要与生产实例共用。

需要：

- Rust/Cargo release 构建环境；
- `curl`、`jq`、`rg`；
- 已登录且可调用的真实 Agent。下面以 `xiaoo` 为例；
- 能以 root 启动 AcTrail。

命令默认从仓库根目录执行。每段 `curl` 都是对应 Web 操作的等价方式；修改状态的
Web 操作和 `curl` 二选一，不要对同一个实例重复执行。

### 步骤 1：构建并安装最新 release

#### CLI 操作

```bash
cd /home/yzh/projects/AcTrail
bash scripts/install-release.sh
```

#### 预期现象

release 构建成功，`target/release/` 下存在 `actraild`、`actrailctl`、
`actrailviewer` 和 `actrailweb`，安装脚本最后打印二进制与官方插件的安装位置。

### 步骤 2：初始化、清理并启动 daemon

#### CLI 操作

```bash
REPO="$(pwd -P)"
WORK=/tmp/actrail-otel-jsonl-manual
BIN="$REPO/target/release"

mkdir -p "$WORK"
printf 'status=ready\nstatus=waiting\n' > "$WORK/search.txt"
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
cat > "$WORK/actraild.patch.toml" <<EOF
[agent_invocation]
commands = ["$XIAOO_BIN"]

[plugins.discovery]
directory = ["$REPO/examples/plugins/builtin"]

[file_observation.collection.read]
observed = true

[file_observation.bulk_read]
enabled = false
EOF

sudo -E "$BIN/actrailctl" --config "$WORK/actraild.conf" \
  init -f --patch "$WORK/actraild.patch.toml"
sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" stop
sudo -E "$BIN/actrailctl" --config "$WORK/actraild.conf" clean
sudo -E "$BIN/actraild" --config "$WORK/actraild.conf" start
```

#### 预期现象

四条 AcTrail 命令均成功。最后一条打印 daemon pid 和 control socket；测试配置中的
插件发现目录指向当前仓库的 `examples/plugins/builtin`。

### 步骤 3：启动 Web 并打开 Plugins 页面

#### CLI 操作

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

#### 预期现象

终端打印 `actrailweb listening on http://127.0.0.1:18080`。浏览器打开
<http://127.0.0.1:18080>，进入 `Plugins` 页面后可以看到
`Installed Plugins` 和 `Plugin candidates`。

### 步骤 4：通过 Web 发现并加载 otel-jsonl

#### Web 操作

1. 在 `Plugin candidates` 标题右侧点击 `Refresh`；
2. 展开 `otel-jsonl`，确认 `Runtime` 为 `Available`、候选状态为 `Unloaded`；
3. 点击 `Load plugin`；
4. 在 `Runtime instance name` 输入 `manual.otel-jsonl`；
5. 再点击弹窗中的 `Load plugin`。

#### 等价 curl

```bash
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS "$WEB_BASE/api/plugins/catalog" > "$WORK/catalog.json"
PACKAGE_QUERY="$(
  jq -er '
    [.packages[] | select(
      .plugin_id == "otel-jsonl"
      and .runtime == "builtin"
      and .activation_ready == true
    )]
    | if length == 1 then .[0].package_key | @uri
      else error("expected one ready otel-jsonl package") end
  ' "$WORK/catalog.json"
)"

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data '{"instance_id":"manual.otel-jsonl"}' \
  "$WEB_BASE/api/plugins/catalog/load?package=$PACKAGE_QUERY" |
  jq '.plugin | {instance_id, plugin_id, runtime, state}'
```

#### 预期现象

候选从 `Plugin candidates` 移到 `Loaded plugin instances`。实例 ID 为
`manual.otel-jsonl`，状态显示 `Active`，runtime 显示 `builtin`。

### 步骤 5：通过 Web 选择 File exporter

#### Web 操作

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

#### 等价 curl

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

#### 预期现象

测试配置后页面显示 `Test passed — ready to update`；更新后显示
`Runtime configuration updated.`。重新展开配置时 `Exporter` 仍为 `File`，且
JSON-RPC 配置不显示。validate API 返回 `"valid": true`。

### 步骤 6：启动真实 Agent 并检查文件导出

#### CLI 操作

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
    "$XIAOO_BIN" --cli run --tools grep --max-turns 4 \
    --prompt "Use grep to find status=ready in $WORK/search.txt, include line numbers, and summarize the results." \
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

#### 预期现象

Agent 成功完成文本任务，launch 输出包含唯一的 `trace trace-N entered Active`。viewer 中能
看到真实 Agent 产生的多类 action；文件导出结果只包含：

```text
command.invocation
file.read
process.exec
```

### 步骤 7：通过 CLI 启动 JRPC 接收端

#### CLI 操作

在第三个终端执行：

```bash
cd /home/yzh/projects/AcTrail
WORK=/tmp/actrail-otel-jsonl-manual

python3 tests/e2e/regression/otel_jsonl_action_filter/receiver.py \
  --output "$WORK/rpc.otlp.jsonl" \
  --fail-next 1 \
  --delay-next-ms 750
```

`--fail-next 1` 让接收端第一次请求返回 HTTP 503；`--delay-next-ms 750` 让下一次
请求等待 750ms 后再响应。这两个参数只用于手动验证重试；只验证正常导出时可以省略。

#### 预期现象

接收端启动后打印：

```text
endpoint=http://127.0.0.1:<动态端口>/rpc
output=/tmp/actrail-otel-jsonl-manual/rpc.otlp.jsonl
```

保持该终端运行，并复制实际打印的 endpoint。

### 步骤 8：通过 Web 切换到 JSON-RPC exporter

#### Web 操作

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

#### 等价 curl

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

#### 预期现象

配置验证通过并显示 `Runtime configuration updated.`。重新展开配置时
`Exporter` 仍为 `JSON-RPC 2.0 over HTTP(S)`，文件配置不显示。

### 步骤 9：再次启动真实 Agent 并检查 JRPC 请求

#### CLI 操作

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
    "$XIAOO_BIN" --cli run --tools grep --max-turns 4 \
    --prompt "Use grep to find status=ready in $WORK/search.txt, include line numbers, and summarize the results." \
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

#### 预期现象

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

### 步骤 10：通过 Web 检查插件状态

#### Web 操作

1. 在 `Plugin candidates` 标题右侧点击 `Refresh`；
2. 在 `Loaded plugin instances` 展开 `manual.otel-jsonl`；
3. 检查状态为 `Active`；
4. 检查 `Records` 显示 `0 dropped`；
5. 等待 `Queue` 排空，检查 `Last error` 为 `none`、`Warnings` 为 `none`。

#### 等价 curl

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

#### 预期现象

实例保持 `active`，`observed_records` 大于零，`dropped_records` 为 `0`；队列最终为
`0/1024`，`last_error` 为空，warnings 为空。

### 步骤 11：通过 Web 卸载并清理环境

#### Web 操作

1. 在 `Loaded plugin instances` 找到 `manual.otel-jsonl`；
2. 点击 `Unload`；
3. 在确认弹窗中点击 `Unload instance`。

#### 等价 curl

```bash
WEB_BASE=http://127.0.0.1:18080
INSTANCE=manual.otel-jsonl

curl -fsS -X POST \
  "$WEB_BASE/api/plugins/runtime/unload?instance_id=$INSTANCE" |
  jq
```

#### CLI 清理

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

#### 预期现象

Web 中运行实例消失，`otel-jsonl` 重新出现在候选列表；receiver、Web 和 daemon
全部停止，手动测试产生的临时文件被删除。
