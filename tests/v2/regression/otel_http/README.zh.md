# OTEL/HTTP V2 回归

# Quick Run

在仓库根目录以 root 执行：

```bash
python3 tests/v2/regression/test_all.py \
  --case otel_http \
  --color never
```

公共 V2 runner 会先执行 `scripts/install-release.sh`，再创建隔离工作目录，启动真实
`actraild`、`actrailweb`、builtin `otel-http` 插件和本地 OTLP/HTTP JSON
receiver，最后通过 `actrailctl launch` 运行一轮真实 xiaoO 对话。运行前需要确保
`xiaoo` 位于 `PATH`，或通过 `XIAOO_E2E_BINARY` 指定可执行文件，并且 xiaoO 的
provider 配置可用。冷缓存机器可能长时间
停留在 release、TLS runtime 或 WASM 插件编译阶段；只要 `cargo`/`rustc` 仍在占用
CPU，就不是测试卡死。

只运行该 case 并显示详细过程：

```bash
python3 tests/v2/regression/otel_http/run_e2e.py \
  --color never
```

# 步骤摘要

1. 检查 release binaries、官方 `otel-http` 插件包和 root/eBPF 测试权限。
2. 在隔离目录初始化、清理并启动 AcTrail，同时启动 `actrailweb` 和本地
   OTLP/HTTP JSON receiver。
3. 发现并加载 builtin `otel-http`，检查配置 schema 和官方安全默认值。
4. 只启用 `process.exec`、`process.exit`，设置 `metadata-only`，并配置长批次超时。
5. 通过 `actrailctl launch` 运行一轮真实 xiaoO 对话，确认批次没有提前发送。
6. 更新插件配置触发旧 consumer 的 `finish`，确认尾批次发送到 receiver。
7. 验证两个 action 均为终态且只出现一次，并验证 metadata-only 出境约束。
8. 验证 OTLP `traceId` 是 UUIDv4 格式的 128-bit 标识，并与 SQLite 中持久化的
   `otel_trace_id` 一致；本机数字 `trace_id` 仍保留为资源属性。
9. 卸载插件，停止 Web、daemon 和 receiver，清理隔离测试数据。

# 手动测试

以下命令完整复现自动测试。所有命令均从仓库根目录、在同一个 root shell 中执行，
以便保留步骤间的环境变量和后台进程 PID。

## 步骤1：检查测试前提并构建 release

### 手动指令

```bash
test "$(id -u)" -eq 0
command -v curl >/dev/null
command -v jq >/dev/null
command -v python3 >/dev/null
test -f examples/plugins/builtin/otel-http/otel-http.plugin.toml
test -f examples/plugins/builtin/otel-http/otel-http.config.toml
test -f examples/plugins/builtin/otel-http/otel-http.config.v1.schema.json

bash scripts/install-release.sh

test -x target/release/actraild
test -x target/release/actrailctl
test -x target/release/actrailviewer
test -x target/release/actrailweb
```

### 预期结果

当前用户为 root；官方插件资产存在；release 安装成功；四个测试所需二进制均可执行。
首次运行可能需要较长时间编译 release、TLS runtime 和官方 WASM 插件。

## 步骤2：创建隔离配置并启动 AcTrail

### 手动指令

```bash
REPO="$(pwd -P)"
BIN="$REPO/target/release"
WORK=/tmp/actrail-otel-http-manual

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
directory = "$REPO/examples/plugins/builtin"
EOF

"$BIN/actraild" --config "$WORK/actraild.conf" \
  init -f --patch "$WORK/actraild.patch.toml"
"$BIN/actraild" --config "$WORK/actraild.conf" stop
"$BIN/actrailctl" --config "$WORK/actraild.conf" clean
"$BIN/actraild" --config "$WORK/actraild.conf" start
```

### 预期结果

所有命令成功；daemon 打印 pid 和 control socket；socket、pid、日志、SQLite 和
export 路径全部位于 `/tmp/actrail-otel-http-manual`。本手册不读取或清理系统默认的
`/var/lib/actrail` 数据。

## 步骤3：启动本地 receiver 和 Web

### 手动指令

创建一个只接受 `/v1/traces` OTLP JSON 请求、并持续保存接收结果的本地 receiver：

```bash
cat > "$WORK/receiver.py" <<'PY'
import json
import signal
import sys
import threading
from pathlib import Path

from tests.v2.regression.otel_http.receiver import OtlpHttpReceiver

output = Path(sys.argv[1])
stopped = threading.Event()
receiver = OtlpHttpReceiver()

signal.signal(signal.SIGINT, lambda *_: stopped.set())
signal.signal(signal.SIGTERM, lambda *_: stopped.set())
receiver.start()
output.write_text("[]\n", encoding="utf-8")
print(receiver.endpoint, flush=True)

try:
    while not stopped.wait(0.2):
        output.write_text(
            json.dumps(receiver.documents(), indent=2) + "\n",
            encoding="utf-8",
        )
finally:
    output.write_text(
        json.dumps(receiver.documents(), indent=2) + "\n",
        encoding="utf-8",
    )
    receiver.stop()
PY

PYTHONPATH="$REPO" python3 "$WORK/receiver.py" \
  "$WORK/receiver-documents.json" \
  > "$WORK/receiver.log" 2>&1 &
RECEIVER_PID=$!

for _ in $(seq 1 50); do
  grep -q '^http://' "$WORK/receiver.log" && break
  sleep 0.1
done
RECEIVER_ENDPOINT="$(grep -m1 '^http://' "$WORK/receiver.log")"
case "$RECEIVER_ENDPOINT" in
  http://127.0.0.1:*/v1/traces) ;;
  *) printf 'receiver did not start: %s\n' "$RECEIVER_ENDPOINT" >&2; exit 1 ;;
esac

"$BIN/actrailweb" \
  --config "$WORK/actraild.conf" \
  --addr 127.0.0.1 \
  --port 18080 \
  > "$WORK/actrailweb.log" 2>&1 &
WEB_PID=$!

for _ in $(seq 1 50); do
  grep -q '^actrailweb listening on ' "$WORK/actrailweb.log" && break
  sleep 0.1
done

WEB_BASE=http://127.0.0.1:18080
curl -fsS "$WEB_BASE/api/plugins/catalog" > "$WORK/catalog.json"
```

### 预期结果

receiver 打印类似 `http://127.0.0.1:PORT/v1/traces` 的临时 endpoint；
`actrailweb` 监听 `127.0.0.1:18080`；catalog 请求成功并返回 JSON。

## 步骤4：发现、加载并配置 otel-http

### 手动指令

```bash
INSTANCE=manual.otel-http

jq -e '
  .packages[]
  | select(
      .package_key == "otel-http"
      and .plugin_id == "otel-http"
      and .runtime == "builtin"
      and .activation_ready == true
    )
' "$WORK/catalog.json"

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data '{"instance_id":"manual.otel-http"}' \
  "$WEB_BASE/api/plugins/catalog/load?package=otel-http" \
  > "$WORK/plugin-load.json"

jq -e '
  .plugin
  | select(
      .instance_id == "manual.otel-http"
      and .plugin_id == "otel-http"
      and .runtime == "builtin"
      and .state == "active"
    )
' "$WORK/plugin-load.json"

curl -fsS \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  > "$WORK/config-document.json"

jq -e '
  .available == true
  and .editable == true
  and .schema.properties.attribute_mode.default == "metadata-only"
  and .schema.properties.action_kinds.additionalProperties == false
  and .schema.properties.action_kinds.properties.default.const == false
  and .config.attribute_mode == "metadata-only"
  and .config.action_kinds.default == false
' "$WORK/config-document.json"

jq --arg endpoint "$RECEIVER_ENDPOINT" '
  {
    config: (
      .config
      | .endpoint = $endpoint
      | .allow_insecure = true
      | .encoding = "json"
      | .compression = "none"
      | .attribute_mode = "metadata-only"
      | .queue_capacity = 128
      | .batch_max_spans = 4096
      | .batch_timeout_ms = 60000
      | .connect_timeout_ms = 250
      | .request_timeout_ms = 500
      | .retry_max_attempts = 1
      | .retry_backoff_ms = 1
      | .shutdown_flush_deadline_ms = 3000
      | .action_kinds = (.action_kinds | with_entries(.value = false))
      | .action_kinds."process.exec" = true
      | .action_kinds."process.exit" = true
    )
  }
' "$WORK/config-document.json" > "$WORK/config-request.json"

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config/validate?instance_id=$INSTANCE" \
  | jq -e '.valid == true'

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/config-request.json" \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  > "$WORK/config-updated.json"

jq -e '
  .config.allow_insecure == true
  and .config.encoding == "json"
  and .config.attribute_mode == "metadata-only"
  and .config.action_kinds."process.exec" == true
  and .config.action_kinds."process.exit" == true
  and ([
    .config.action_kinds
    | to_entries[]
    | select(.key != "process.exec" and .key != "process.exit")
    | .value
  ] | all(. == false))
' "$WORK/config-updated.json"
```

### 预期结果

catalog 中存在 activation-ready 的 `otel-http`；加载后实例状态为 `active`；schema
保持安全默认值；本地明文 endpoint 通过显式 `allow_insecure=true` 被接受；仅
`process.exec`、`process.exit` 被启用；批次上限为 4096、超时为 60 秒。

## 步骤5：启动真实 trace 并确认没有提前发送

### 手动指令

```bash
CASE_MARKER="OTEL_HTTP_MANUAL_$(python3 -c 'import secrets; print(secrets.token_hex(6))')"
ANSWER_MARKER="A$(python3 -c 'import secrets; print(secrets.token_hex(5))')"
XIAOO_BIN="${XIAOO_E2E_BINARY:-$(command -v xiaoo)}"
test -x "$XIAOO_BIN"
LAUNCH_OUTPUT="$(
  "$BIN/actrailctl" --config "$WORK/actraild.conf" \
    launch \
    --name "$CASE_MARKER" \
    --host-ebpf required \
    --seccomp-notify auto \
    -- \
    "$XIAOO_BIN" \
    --cli run \
    --no-tools \
    --max-turns 1 \
    --prompt "Reply with exactly \"$ANSWER_MARKER\" and nothing else. Do not use tools." \
    2>&1
)"
printf '%s\n' "$LAUNCH_OUTPUT"
printf '%s\n' "$LAUNCH_OUTPUT" | grep -F "$ANSWER_MARKER"

TRACE_IDS="$(
  printf '%s\n' "$LAUNCH_OUTPUT" |
    sed -n 's/.*trace trace-\([0-9][0-9]*\) entered Active.*/\1/p'
)"
test "$(printf '%s\n' "$TRACE_IDS" | sed '/^$/d' | wc -l)" -eq 1
TRACE_ID="$(printf '%s\n' "$TRACE_IDS" | tail -n1)"

TRACE_STATE=
for _ in $(seq 1 30); do
  TRACE_STATE="$(
    "$BIN/actrailviewer" --config "$WORK/actraild.conf" \
      --output-format json traces |
      jq -r --argjson trace_id "$TRACE_ID" '
        .traces[]
        | select(.trace_id_raw == $trace_id)
        | "\(.state)/\(.health)"
      '
  )"
  case "$TRACE_STATE" in
    Exited/Clean|Completed/Clean) break ;;
  esac
  sleep 1
done
case "$TRACE_STATE" in
  Exited/Clean|Completed/Clean) ;;
  *) printf 'trace did not finish cleanly: %s\n' "$TRACE_STATE" >&2; exit 1 ;;
esac

sleep 1
EARLY_SPANS="$(
  jq --arg marker "$CASE_MARKER" '
    [
      .[]
      | .resourceSpans[]?
      | select(any(
          .resource.attributes[]?;
          .key == "actrail.trace.display_name"
          and .value.stringValue == $marker
        ))
      | .scopeSpans[]?
      | .spans[]?
    ]
    | length
  ' "$WORK/receiver-documents.json"
)"
test "$EARLY_SPANS" -eq 0
```

### 预期结果

真实 xiaoO 对话成功退出并输出 answer marker；输出中恰好出现一个
`trace trace-N entered Active`；trace 最终为
`Exited/Clean` 或 `Completed/Clean`；receiver 中该 marker 的 span 数量仍为 0，
证明未达到批次上限或超时前不会提前发送。

## 步骤6：触发 finish 并验证 OTLP 数据

### 手动指令

```bash
curl -fsS \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  > "$WORK/config-before-finish.json"

jq '{config: (.config | .batch_timeout_ms = 59000)}' \
  "$WORK/config-before-finish.json" \
  > "$WORK/config-finish-request.json"

curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @"$WORK/config-finish-request.json" \
  "$WEB_BASE/api/plugins/runtime/config?instance_id=$INSTANCE" \
  | jq -e '.config.batch_timeout_ms == 59000'

for _ in $(seq 1 30); do
  jq --arg marker "$CASE_MARKER" '
    [
      .[]
      | .resourceSpans[]?
      | select(any(
          .resource.attributes[]?;
          .key == "actrail.trace.display_name"
          and .value.stringValue == $marker
        ))
      | .scopeSpans[]?
      | .spans[]?
    ]
  ' "$WORK/receiver-documents.json" > "$WORK/marker-spans.json"
  test "$(jq 'length' "$WORK/marker-spans.json")" -ge 2 && break
  sleep 1
done

python3 - "$WORK/marker-spans.json" "$WORK/data/actrail.sqlite" "$TRACE_ID" <<'PY'
import json
import re
import sqlite3
import sys
from collections import Counter
from pathlib import Path

spans = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
database = Path(sys.argv[2])
local_trace_id = int(sys.argv[3])
allowed_keys = {
    "actrail.action.id",
    "actrail.action.kind",
    "actrail.action.status",
    "actrail.action.completeness",
    "actrail.process.id",
    "actrail.action.valid",
    "process.parent.identity_state",
}

def attribute(span, key):
    for item in span.get("attributes", []):
        if item.get("key") != key:
            continue
        value = item.get("value", {})
        return value.get("stringValue", value.get("intValue"))
    return None

kinds = [attribute(span, "actrail.action.kind") for span in spans]
action_ids = [attribute(span, "actrail.action.id") for span in spans]
statuses = [attribute(span, "actrail.action.status") for span in spans]
unexpected = sorted({
    item.get("key")
    for span in spans
    for item in span.get("attributes", [])
    if item.get("key") not in allowed_keys
})

assert Counter(kinds) == Counter({"process.exec": 1, "process.exit": 1})
assert len(action_ids) == len(set(action_ids)) == 2
assert all(status in {"success", "error", "unknown"} for status in statuses)
assert all(span.get("name") == kind for span, kind in zip(spans, kinds))
assert unexpected == []

wire_ids = {span.get("traceId") for span in spans}
assert len(wire_ids) == 1
wire_id = next(iter(wire_ids))
assert isinstance(wire_id, str) and re.fullmatch(r"[0-9a-f]{32}", wire_id)
assert wire_id != "0" * 32
assert wire_id != f"{local_trace_id:032x}"
assert wire_id[12] == "4" and wire_id[16] in "89ab"
with sqlite3.connect(database) as connection:
    stored = connection.execute(
        "SELECT lower(hex(otel_trace_id)), length(otel_trace_id) "
        "FROM traces WHERE trace_id = ?",
        (local_trace_id,),
    ).fetchone()
assert stored == (wire_id, 16)

print({
    "spans": len(spans),
    "kinds": dict(sorted(Counter(kinds).items())),
    "action_ids_unique": True,
    "terminal": True,
    "metadata_only": True,
    "otel_trace_id": wire_id,
    "persistent_identity": True,
})
PY

curl -fsS "$WEB_BASE/api/plugins/runtime" |
  jq -e --arg instance "$INSTANCE" '
    .plugins[]
    | select(
        .instance_id == $instance
        and .state == "active"
        and .dropped_records == 0
      )
  '
```

### 预期结果

更新配置成功并触发旧 consumer 的 `finish`；receiver 收到 2 个 span；
`process.exec=1`、`process.exit=1`，action ID 唯一，状态全部为终态，span 名称等于
action kind，没有超出 metadata-only 白名单的属性；所有 span 使用同一个 UUIDv4
`traceId`，且该值与 SQLite 的 16-byte `otel_trace_id` 一致；插件保持 `active` 且
`dropped_records=0`。

## 步骤7：清理测试进程和隔离数据

### 手动指令

```bash
curl -fsS -X POST \
  "$WEB_BASE/api/plugins/runtime/unload?instance_id=$INSTANCE" |
  jq -e '.plugin.state == "stopped"'

kill "$WEB_PID"
wait "$WEB_PID" || true

"$BIN/actraild" --config "$WORK/actraild.conf" stop
"$BIN/actrailctl" --config "$WORK/actraild.conf" clean

kill "$RECEIVER_PID"
wait "$RECEIVER_PID" || true

case "$WORK" in
  /tmp/actrail-otel-http-manual) rm -rf -- "$WORK" ;;
  *) printf 'refusing to remove unexpected path: %s\n' "$WORK" >&2; exit 1 ;;
esac
```

### 预期结果

插件进入 `stopped`；Web、daemon 和 receiver 均退出；AcTrail 清理命令只操作隔离
配置，最后删除固定的 `/tmp/actrail-otel-http-manual` 手动测试目录。

# 覆盖范围与非目标

本用例验证 builtin OTEL/HTTP 的插件发现、配置安全策略、真实 action 输入、批次
flush、终态 one-shot 和 metadata-only 出境约束。它不依赖外部 Collector、容器或
Agent CLI，也不替代以下专项验收：

- TLS/mTLS 证书分发、轮换和真实 Collector 互操作；
- WAL、at-least-once 或其他可靠投递语义；
- 不同部署形态的网络故障和生命周期故障矩阵。
