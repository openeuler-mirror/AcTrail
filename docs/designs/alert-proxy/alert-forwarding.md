# 告警转发组件设计

## 1. 系统边界

`actraild-alert-proxy` 是独立进程。
它接收 `actraild` 已成功持久化的标准化告警，并转发给所有订阅条件匹配的外部连接。

```text
actraild alert sources
  → builtin alert-forwarding plugin
  → bounded daemon queue
  → persistent AF_UNIX connection
  → actraild-alert-proxy
  → subscriber registry
  → per-subscriber bounded queue
  → external TCP subscriber
```

`actraild-alert-proxy` 不读取主 Storage，不参与 trace 关联，不决定告警是否成立，也不改变原告警的持久化结果。
外部订阅者故障只影响对应订阅连接。
proxy 故障只关闭告警外发能力，不反向破坏告警产生、告警持久化或 daemon 其他服务。

## 2. C4 Container 与 Component

### 2.1 actraild Container

- `AlertIngress`：接纳并持久化标准告警。
- `SandboxAlertPipeline`：把 Sandbox 资源告警写入独立 Sandbox Alert DB，并在事务提交后生成标准外发告警。
- `AlertForwardingPlugin`：持有类别选择和连接门控后的有效启用状态。
- `AlertProxyLink`：以有界队列和单条持久 AF_UNIX 连接发送标准告警。
- `AlertProxySupervisor`：探测 proxy、拉起配置的二进制并完成首次连接。
- `PluginConfigBridge`：把 builtin plugin 的配置、校验和更新暴露给现有 Web 控制面。

### 2.2 actraild-alert-proxy Container

- `DaemonIngressListener`：限制待握手连接数量，只允许一个活动 `actraild` producer，接收内部二进制告警帧。
- `AlertBroadcaster`：通过有界队列接收 daemon 告警，取得订阅会话快照，并以非阻塞方式向匹配会话投递。
- `SubscriberListener`：接受外部 TCP 连接，并为每个连接创建 `SubscriberSession`。
- `SubscriberRegistry`：保存已握手的活动 session，并在连接关闭时回收。
- `SubscriberSession`：处理鉴权、订阅、Heartbeat、过滤和 JSON 推送。

## 3. actraild builtin plugin

builtin plugin 的稳定实例 ID 为 `builtin.alert-forwarding`，purpose 为 `alert-consumer`。
它不进入 ObservationConsumer、ControlDecider 或 sandbox observation 路由。

插件配置包含：

- `enabled`：请求启用告警外发；
- `categories`：允许发送的告警类别；
- `all_categories`：显式允许全部类别。

`categories` 对应标准告警的 `cat`。
空 `categories` 且 `all_categories=false` 表示不转发任何告警。
`all_categories=true` 时 `categories` 必须为空。
类别只校验非空、长度、数量和允许字符，不依赖静态类别全集。

插件分别维护 requested configuration 和 effective state。
只有 proxy 进程可用且 daemon ingress 连接已完成握手时，effective `enabled` 才能为 `true`。

Web 读取配置时返回有效 `enabled`。
连接断开后，运行时立即把 requested 和 effective `enabled` 都降为 `false`，并在低频控制路径更新 plugin config file。
类别配置保留，便于下一次显式启用时复用。

Web 提交 `enabled=true` 时执行以下动作：

1. 校验配置结构和类别集合；
2. 探测 daemon ingress socket；
3. socket 不可用时按配置拉起 `actraild-alert-proxy`；
4. 等待 proxy ingress ready；
5. 建立连接并完成握手；
6. 原子发布类别过滤快照；
7. 原子写入 builtin plugin config file；
8. 把有效 `enabled` 设置为 `true`。

任一步失败时更新失败，有效 `enabled` 保持 `false`。
`enabled=false` 只关闭 daemon 的转发门，不要求终止 proxy 进程。

## 4. actraild 启动行为

daemon 的 `[alert_forwarding]` 子配置只控制 proxy executable、proxy config path、builtin plugin config path、本机 socket、队列、I/O timeout、Heartbeat interval、启动等待周期和线程栈。
初始 requested enabled 和类别选择只来自 builtin plugin config file。
当前 profile 的 builtin plugin config path 为 `/etc/actrail/plugins/alert-forwarding/alert-forwarding.config.json`。
文件不存在时，daemon 在该路径创建 `enabled=false` 的完整配置；目录创建、写入或原子替换失败使启动失败。

plugin config请求启用时，`actraild` 在开放 control service 前执行 proxy 探测、必要的进程拉起和 ingress 握手。
配置、路径、进程创建、初始连接、握手或link owner创建错误使 daemon 启动失败。
启动时请求启用表示proxy连接是必需依赖，不允许静默降级。

proxy 是否存在以 daemon ingress 握手为准，不使用进程名或 `pgrep` 判断。
连接已有 proxy 时，daemon 只借用连接。
由 daemon 拉起 proxy 时，daemon 保存 child handle，并使用 `try_wait` 回收退出状态。

运行中连接断开时关闭 requested 和 effective转发状态，并丢弃旧 connection generation 中尚未发送的外发副本。
daemon 不在告警热路径同步拉起或同步重连 proxy。
下一次显式提交 `enabled=true` 时重新执行探测、拉起和连接。

## 5. 告警来源与标准化

### 5.1 通用 AlertIngress

插件告警和 daemon enforcement 告警只有在主 Storage 返回 `Stored` 后才进入转发插件。
`DuplicateSuppressed`、`RejectedTraceToken` 和存储失败不转发。

`AlertIngress` 在内存中保留注册时的轻量 definition metadata。
转发时使用已校验 draft、检测时间和 definition metadata 构造标准告警，不为外发增加 SQLite 回读。
`cat` 取 `AlertDefinition.kind`。
`description` 取 `AlertDefinition.title`。

### 5.2 sandbox resource alert 边界

`sandbox-resource-alert` 产生的 `SandboxAlert` 没有 trace ID，并且不进入主告警存储。
它通过有界、非阻塞的提交边界进入独立 Sandbox Alert DB。
独立数据库事务提交成功后，daemon 将告警转换为 sandbox source 的标准告警，并交给同一个 builtin forwarding plugin。

Sandbox Alert DB 写入失败时不外发对应告警。
forwarding disabled、类别不匹配、queue 满或 proxy 断开时只丢弃外发副本，不改变已提交的数据库记录，也不反向影响 Hand observation ingestion。

禁止为 Sandbox 告警伪造 `source.trid`。

## 6. 标准告警模型

标准告警包含：

- `id`：proxy 生成的 UUID；
- `ts`：检测时间的 13 位 Unix epoch 毫秒；
- `source`：互斥的真实来源；trace 告警包含 `trid`，Sandbox 告警包含 `sandbox` 对象；
- `s`：外部严重级别；
- `cat`：告警类别；
- `description`：可选描述；
- `labels`：预留对象，当前为空；
- `extras`：原始业务 JSON；非 object payload 包装为 `{ "value": <payload> }`。

trace source 保持以下形状：

```json
{
  "source": {
    "trid": "trace-42"
  }
}
```

Sandbox resource source 使用以下形状：

```json
{
  "source": {
    "sandbox": {
      "gateway_id": 7,
      "sb_id": 3,
      "boot_id": "550e8400-e29b-41d4-a716-446655440000"
    }
  }
}
```

进程 I/O 告警在 `sandbox` 对象中增加 `process`：

```json
{
  "pid": 123,
  "start_time_ticks": 456,
  "executable_name_hex": "7869616f6f0000000000000000000000"
}
```

进程二进制名按固定 16-byte 原值编码为 32 位小写十六进制，不执行有损 UTF-8 转换。

外部严重级别统一为 `info`、`warning`、`critical`：

- internal `informational`、`low` → `info`；
- internal `medium`、`high` → `warning`；
- internal `critical` → `critical`。

## 7. 配置分层

`actraild` 只读取 `[alert_forwarding]` 和 builtin plugin 配置。
它不知道 proxy 的 subscriber listener、鉴权 token、subscriber queue 或 session 文件结构。

`actraild-alert-proxy` 使用独立 TOML 配置，包含：

- daemon ingress UDS path、权限、I/O timeout、frame limit；
- subscriber bind address、连接上限、backlog、I/O timeout、frame limit；
- heartbeat interval、peer idle timeout；
- per-subscriber queue capacity、worker stack；
- 允许的 opaque bearer tokens。

subscriber listener 的当前部署边界是 loopback 或受保护的内部网络。
跨不可信网络暴露时必须在 listener 前部署 TLS terminator。
proxy 默认拒绝非 loopback bind，只有显式配置 `allow_insecure_remote=true` 才允许在没有内建 TLS 的情况下 bind 非 loopback 地址。

所有路径、容量、周期、地址和线程栈都有配置入口。
配置非法时进程启动失败。

builtin plugin 使用独立 JSON config file。
Web 更新在完成连接门控后原子替换该文件，再发布运行时快照。
单一 `AlertForwardingStateOwner` 串行处理 Web 更新、启动降级和 link 断连状态提交。

## 8. 性能与故障边界

daemon 到 proxy 使用单条持久 AF_UNIX stream 和紧凑二进制 frame。
每条告警不重新建连，不使用 JSON，不轮询数据库，也不引入共享内存 ring、eventfd 或重型哈希。

daemon 告警热路径只执行：

1. 读取 effective enabled 原子状态；
2. 读取不可变类别过滤快照；
3. 对有界 queue 执行 `try_send`。

`AlertIngress` 在解析 payload 和构造外发对象前先执行 enabled/category 预检。
预检未通过时不进行 JSON 二次解析或字符串分配。
最终 `try_publish` 再次检查 enabled 和类别快照，覆盖预检后的并发配置变化。

queue 满时丢弃当前外发副本，不阻塞告警持久化。
每个 queued item携带 connection generation。
disable 或断线后，writer丢弃旧 generation item，避免重新启用时发送陈旧副本。
link 只有在失败 generation 仍等于 active generation 时才能提交 disabled。
旧 generation 的迟到 EOF、HUP 或 ack timeout 不得覆盖新连接状态。

proxy daemon ingress 只向有界 broadcast queue 执行 `try_send`，不在 producer connection owner 内编码外部 JSON。
queue 满时只丢弃当前外发副本，不阻塞 HeartbeatAck。

proxy 广播线程只在 registry lock 内取得 session 快照。
实际过滤与 `try_send` 不持有 registry lock。
proxy为一条告警只生成一次UUID并只编码一次长度前缀JSON frame。
匹配session的queue共享同一个 `Arc<[u8]>`。
每个 subscriber 使用独立有界 queue 和 writer owner。
慢 subscriber 的 queue 满时只关闭该 subscriber。

运行中协议错误、EOF、timeout、queue full 和单连接线程错误均 fail-local。
启动配置、bind、权限、鉴权配置和初始必需连接错误均 fail-fast。
