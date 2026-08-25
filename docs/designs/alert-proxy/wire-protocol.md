# 告警代理传输协议

## 1. actraild 到 proxy

### 1.1 Transport

daemon ingress 使用持久 AF_UNIX stream。
socket path 由两端配置，必须相同。
proxy 同一时刻只接受一个活动 producer。

每个 frame 使用以下 header：

```text
magic       4 bytes  "ATAP"
version     1 byte   2
message     1 byte
reserved    2 bytes  0
length      4 bytes  big-endian u32
payload     length bytes
```

frame size limit 由两端配置，daemon 的发送上限不得大于 proxy 的接收上限。

message code如下：

| Code | Message | Direction |
| --- | --- | --- |
| `0x01` | `ProducerHello` | daemon → proxy |
| `0x02` | `ProducerWelcome` | proxy → daemon |
| `0x03` | `ProducerReject` | proxy → daemon |
| `0x10` | `ForwardAlert` | daemon → proxy |
| `0x20` | `Heartbeat` | daemon → proxy |
| `0x21` | `HeartbeatAck` | proxy → daemon |

### 1.2 Handshake

daemon 建连后发送 `ProducerHello`。
payload是 big-endian `u32 daemon_pid`。

proxy要求 `daemon_pid` 等于 `SO_PEERCRED.pid`，并按配置校验允许的 uid和gid。
socket parent、owner和mode由proxy配置，启动时设置完成。

proxy 校验协议、peer credentials、完整 ready gate 和当前 producer 槽位后返回空 payload `ProducerWelcome`。
完成 welcome 前，连接不属于有效 producer。

第二个 producer 在已有活动 producer 时收到 `ProducerReject` 并被关闭。
`ProducerReject` payload为 `u16 code_length` 加 UTF-8 error code bytes。

### 1.3 Alert 与 Heartbeat

`ForwardAlert` 不包含外部 message ID和labels。
payload按以下顺序编码：

```text
detected_at_ms       u64 big-endian
severity             u8: 1=info, 2=warning, 3=critical
source_kind          u8: 1=trace, 2=sandbox
source               variant payload described below
category_length      u16 big-endian
category             UTF-8 bytes
has_description      u8: 0 or 1
description_length   u16 big-endian, only when has_description=1
description          UTF-8 bytes, only when has_description=1
extras_length        u32 big-endian
extras               UTF-8 JSON object bytes
```

trace source payload：

```text
trace_id_length      u16 big-endian
trace_id             UTF-8 bytes
```

sandbox source payload：

```text
gateway_id           u32 big-endian, non-zero
sb_id                u32 big-endian, non-zero
guest_boot_id        16 raw bytes, non-zero
has_process          u8: 0 or 1
pid                  u32 big-endian, only when has_process=1, non-zero
start_time_ticks     u64 big-endian, only when has_process=1, non-zero
executable_name      16 raw bytes, only when has_process=1
```

source variant 必须互斥。
Sandbox source 不编码 trace ID。
trace ID、category、description和extras的配置上限不得超过对应长度字段的协议上限。
proxy校验 UTF-8、severity、可选标记、字段长度、13位检测时间和extras JSON object。

daemon 按 producer heartbeat interval 发送 `Heartbeat`，不因普通告警流量停止探活。
Heartbeat payload为 big-endian `u64 nonce`。
proxy返回携带相同 nonce的 `HeartbeatAck`。
daemon 同时只允许一个 outstanding Heartbeat。
收到匹配 ack 后才能发送下一次 Heartbeat。
producer ack timeout 从发送 outstanding Heartbeat 时开始计算。
其他 frame不能替代匹配nonce的ack。
Heartbeat 不进入 broadcaster。
daemon link reader持续读取 ack并监测 EOF/HUP。
写失败、EOF、HUP、错误ack或producer ack timeout使 daemon link把 requested和effective enabled置为 `false`。

proxy以任意合法producer frame刷新producer activity。
达到producer idle timeout仍无合法frame时释放producer槽位。
producer idle timeout必须大于producer heartbeat interval和ack timeout。

协议不提供逐告警 ACK。
一次完整写成功但 proxy 在广播前退出时，告警外发副本可能丢失，不影响主 Storage 中的告警记录。
Sandbox 告警外发副本丢失不影响独立 Sandbox Alert DB 中已提交的记录。

## 2. proxy 到 subscriber

### 2.1 Framing

subscriber listener 使用 TCP。
每条消息由 4-byte big-endian `u32` JSON 长度和 UTF-8 JSON payload 组成。
JSON frame 不能依赖一次 `read` 的边界。
当前 proxy 不内建 TLS，默认只允许 loopback bind。
受保护内部网络必须显式允许 insecure remote bind；不可信网络必须在 proxy 前终止 TLS。

连接建立后，第一条消息必须是 handshake request。
未握手连接不能订阅，也不能接收告警。

### 2.2 握手

subscriber → proxy：

```json
{
  "action": "handshake",
  "version": "v1",
  "auth": { "token": "jwt_xxx" },
  "client_id": "hostname-001"
}
```

proxy → subscriber：

```json
{
  "status": "success",
  "session_id": "sess_abc123",
  "heartbeat_interval": 30
}
```

v1 中 `auth.token` 是 opaque bearer token。
proxy 使用轻量逐字节 XOR 累积执行定长比较，不计算哈希，也不声称执行 JWT 签名、issuer、audience 或 expiry 校验。
空 token 集或空 token 使 proxy 启动失败。
部署占位 token 使 proxy 启动失败。
日志不得记录token。

`client_id` 必须非空，并受配置的最大长度限制。
`session_id` 只在当前 proxy 进程生命周期内标识连接。

### 2.3 订阅

subscriber → proxy：

```json
{
  "id": "req_001",
  "action": "subscribe",
  "topics": ["oom.killed"],
  "filter": {
    "severity": ["critical", "warning"],
    "tags": {}
  }
}
```

proxy → subscriber：

```json
{
  "id": "req_001",
  "status": "accepted",
  "subscribed_topics": ["oom.killed"]
}
```

响应返回本次实际接受的 topics，不能返回其他 topic。
再次 subscribe 原子替换当前 session 的订阅快照。

`topics` 只校验非空、长度、数量和允许字符，proxy不维护静态类别全集。
通过语法校验的topics均被接受，并在响应中原样返回。
`topics` 与标准告警 `cat` 精确匹配。
空 topics 表示不订阅任何告警。
`filter.severity` 与标准告警 `s` 精确匹配。
空 severity 表示不按严重级别过滤。
非空 severity 最多包含 `info`、`warning`、`critical` 各一次，并在 session 内压缩为定长位掩码。
`filter.tags` 预留，v1 只接受空 object。
未来 `filter.tags` 匹配推送消息的 `labels`。

### 2.4 告警推送

proxy → subscriber：

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ts": 1755912345678,
  "source": {
    "trid": "trace-id"
  },
  "s": "critical",
  "cat": "oom.killed",
  "description": "Guest OOM kill count increased",
  "labels": {},
  "extras": {}
}
```

`ts` 是检测时间，不是 proxy 接收或发送时间。
`extras` 必须是 JSON object。
业务定制字段只能放入 `extras`，不能扩展顶层骨架。

Sandbox 告警的 `source` 为：

```json
{
  "sandbox": {
    "gateway_id": 7,
    "sb_id": 3,
    "boot_id": "550e8400-e29b-41d4-a716-446655440000",
    "process": {
      "pid": 123,
      "start_time_ticks": 456,
      "executable_name_hex": "7869616f6f0000000000000000000000"
    }
  }
}
```

资源类 Sandbox 告警省略 `process`。

### 2.5 Heartbeat

proxy 按 heartbeat interval 发送 ping，不因告警推送而停止探活。
同一 session 同时只允许一个 outstanding ping：

```json
{
  "action": "ping",
  "nonce": 42,
  "ts": 1755912345678
}
```

subscriber 返回：

```json
{
  "action": "pong",
  "nonce": 42,
  "ts": 1755912345678
}
```

收到任意合法 inbound frame 都刷新 peer activity。
pong必须回显当前 outstanding ping nonce。
没有 outstanding ping的pong或nonce不匹配属于协议错误。
ping发出时启动独立pong deadline。
subscribe和其他合法frame不能清除outstanding ping，也不能延长pong deadline。
pong deadline到达时只关闭当前session。
达到 peer idle timeout 仍未收到合法 frame 时，proxy 只关闭当前 subscriber session。
peer idle timeout 必须大于 heartbeat interval。

### 2.6 Error

握手、鉴权或订阅语义错误时，proxy发送以下 error 后关闭连接。
frame header、长度、UTF-8或JSON解码失败时直接关闭连接。

```json
{
  "status": "error",
  "id": "req_001",
  "code": "invalid_subscription",
  "message": "subscription topics are invalid"
}
```

订阅错误回显request ID，握手错误省略 `id`。
error message 不包含 token、内部路径或其他 secret。
