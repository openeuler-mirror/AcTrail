# 执行隔离采集观测编号式运行时序

本文描述以下完整通路的实际启动动作和运行行为：

```text
Guest workload
  → Guest-only eBPF / procfs
  → actrail-sb
  → AF_VSOCK
  → actrail-vsock-gateway
  → TCP
  → actraild Hand listener
  → sandbox plugin 或独立 Sandbox Evidence DB
```

checked-in deployment profile 假设 `actraild` 与 `actrail-vsock-gateway` 位于同一台 sandbox Host。
gateway 连接 daemon 的 upstream endpoint（当前 profile 默认 `127.0.0.1:9472`）。
`actrail-sb daemon` 位于 Guest，并在制作 microVM 快照前完成采集设施初始化。

快照恢复后，Guest 内的 `actrail-sb connect` CLI 通过 Guest-local control socket 向 daemon 下发 Host CID 与 VSOCK port，并请求建立 gateway 连接。

当前主线 VMM 为 Firecracker。

Host gateway 根据该 microVM 的 VSOCK `uds_path` 与同一 port 形成 `${uds_path}_${port}` listener endpoint。

Cloud Hypervisor 与 native AF_VSOCK 是并列可选 gateway backend。

StratoVirt/Kata 使用 native AF_VSOCK backend，不新增 StratoVirt 专用 gateway transport。
`execution_isolation_firecracker`、`execution_isolation_stratovirt` 与
`execution_isolation_cloud_hypervisor` 分别保留真实 VMM 验收边界，结果不能跨 backend、
CPU 架构、Guest kernel 或 runtime 组合外推。

下文先描述配置概念，再在括号中标注 checked-in deployment profile 的具体默认值。
标注为当前 profile 默认值的 IP、CID、port、文件路径、容量、周期、线程栈和告警阈值可由所属配置或明确的运行时 CLI 参数修改；
协议常量与实现常量会明确标注。
架构不变量是本节明确列出的跨端 endpoint 兼容关系、timeout 大小关系、queue 容量关系、绝对路径要求和协议字段宽度。

本文中的 accept、connection、writer、consumer、collector 和 sender 均为所属进程内部的线程；
它们不会 `fork` 出新的 daemon 进程。
Guest-only eBPF programs 在 Guest kernel 内执行，也不是用户态线程。

## 0. 启动前准备

### 0.1 准备部署配置与资源

**0.1.1** 准备完整的 daemon 配置（当前 profile 路径为 `/etc/actrail/operator.conf`）。
仓库中的 `deploy/execution-isolation/actraild-sandbox-resource-alert.startup.toml`
只包含 `[hand_observation]`、`[sandbox_evidence]`、`[plugins.startup]` 等执行隔离片段。
它必须合并到完整 operator 配置，不能单独作为 `actraild` 配置运行。
control UDS、PID file 和主 Storage 等既有 daemon 配置仍由完整 operator 配置提供。

**0.1.2** 准备资源告警插件的 manifest、schema 和 plugin config；
三者的安装路径必须与 startup fragment 一致（当前 profile 使用以下路径）：

```text
/usr/share/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.plugin.toml
/usr/share/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.v1.schema.json
/etc/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.json
```

**0.1.3** 准备独立 Evidence DB 和 Sandbox Alert DB（当前 profile 使用以下路径）：

```text
/var/lib/actrail/sandbox-evidence.sqlite
/var/lib/actrail/sandbox-alerts.sqlite
```

两个数据库分别使用独立的绝对路径、schema、SQLite connection 和 writer lifecycle。
配置决定是否创建父目录；daemon 用户必须拥有对应目录权限。

**0.1.4** 使用当前 release binary生成 gateway和Guest daemon配置（当前 profile 路径分别为 `/etc/actrail/actrail-vsock-gateway.toml` 与 `/etc/actrail/actrail-sb.toml`）。

Guest daemon配置拥有采集、容量、周期、连接策略、实例锁和 Guest-local control socket。

Host CID 与 VSOCK port 不属于快照内的 daemon静态配置。

它们由快照恢复后的 `actrail-sb connect` CLI 提供。

CLI 提供的 VSOCK port 与 Host backend endpoint表示同一个具体 port（当前 profile 默认 `43182`）。

Firecracker profile由 gateway使用 `listener.uds_path` 与 `listener.port` 形成 `${uds_path}_${port}`。

操作员不手工拼接端口后缀。

StratoVirt profile 使用 gateway native listener 的 Host CID 与 port；Guest 的运行时
connect port 必须与该 native listener port 一致。

gateway `upstream.daemon_address` 必须能够连接 daemon实际 bind的 Hand listener（当前 same-host profile 在两端使用 `127.0.0.1:9472`）。

wildcard bind、其他网卡地址或网络地址转换场景不要求配置字符串相同。

### 0.2 校验容量、周期与跨组件关系

**0.2.1** execution-isolation 的容量和周期均由所属配置控制。
当前 profile 默认值如下：

- actraild Hand listener
  - gateway 连接上限（当前 profile 默认 64）
  - accept 轮询周期（当前 profile 默认 20 ms）
  - connection read 轮询周期（当前 profile 默认 250 ms）
  - idle timeout（当前 profile 默认 15 s）
  - write timeout（当前 profile 默认 1 s）
  - read buffer（当前 profile 默认 65,536 B）
  - connection stack（当前 profile 默认 524,288 B）
- Sandbox Evidence DB
  - writer queue capacity（当前 profile 默认 1024 batch）
  - batch observation limit（当前 profile 默认 1024）
  - transaction batch limit（当前 profile 默认 32）
  - transaction aggregation deadline（当前 profile 默认 250 ms）
  - retention row limit（当前 profile 默认 1,000,000）
  - database capacity limit（当前 profile 默认 1 GiB）
  - shutdown drain budget（当前 profile 默认 10 s）
- resource-alert plugin
  - consumer queue capacity（当前 profile 默认 1024）
  - source-state capacity（当前 profile 默认 4096）
  - alert queue capacity（当前 profile 默认 1024）
  - flush idle interval（当前 profile 默认 1 s）
  - available-memory threshold（当前 profile 默认 512 MiB）
  - per-interval read/write threshold（当前 profile 默认各 256 MiB）
- gateway
  - SB 连接上限（当前 profile 默认 64）
  - per-SB quota（当前 profile 默认 16）
  - outbound queue capacity（当前 profile 默认 1024）
  - upstream Heartbeat interval（当前 profile 默认 5 s）
  - SB idle timeout（当前 profile 默认 15 s）
  - I/O timeout（当前 profile 默认 1 s）
  - reconnect interval（当前 profile 默认 1 s）
- actrail-sb
  - Guest-local control socket（当前 profile 默认 `/run/actrail/actrail-sb-control.sock`）
  - Guest-local control socket mode（当前 profile 默认 `0600`）
  - control request deadline（当前 profile 默认 5 秒）
  - control accepted connection上限（当前 profile 默认 8）
  - control binary frame上限（当前 profile 默认 1024 B；协议最小值为 523 B）
  - control worker stack（当前 profile 默认 262,144 B）
  - root refresh interval（当前 profile 默认 1 s）
  - I/O poll interval（当前 profile 默认 1 s）
  - resource poll interval（当前 profile 默认 1 s）
  - observation queue capacity（当前 profile 默认 1024）
  - batch observation limit（当前 profile 默认 256）
  - max silence interval（当前 profile 默认 5 s）
  - reconnect interval（当前 profile 默认 1 s）
  - worker stack（当前 profile 默认各 524,288 B）
  - diagnostics interval（当前 profile 默认 0，即关闭）
- Guest eBPF maps
  - tracked-process capacity（当前 profile 默认 16,384）
  - pending-I/O capacity（当前 profile 默认 32,768）
  - root-aggregate capacity（当前 profile 默认 4096）

**0.2.2** daemon supervision 配置整个 daemon shutdown 的监督等待预算（当前 profile 的 `shutdown_wait_ms=170100`），它不是 Evidence writer 自身的 drain timeout（当前 profile 默认 10 秒）。

**0.2.3** 跨组件配置必须满足以下关系：

- CLI 下发的目标 port 与 Host backend endpoint 表示同一个具体 port。
- Firecracker 主线中 CLI 向 SB daemon下发 Host CID `2` 与运行时 port，gateway 监听 `${uds_path}_${port}`。
- StratoVirt 中 CLI 向 SB daemon下发 Host CID `2` 与运行时 port，gateway 通过 native AF_VSOCK 在同一 port 监听。
- gateway upstream endpoint能够连接 daemon Hand listener；
- SB `max_silence_interval` 小于 gateway `sb_peer_idle_timeout`；
- gateway `upstream_heartbeat_interval` 小于 daemon `connection_idle_timeout`；
- gateway `outbound_queue_capacity >= max_sb_connections * per_sb_forward_quota`；
- procfs root、SB instance lock、SB control UDS、Evidence DB、Sandbox Alert DB 与 VMM Unix endpoint 使用绝对路径。
  同一 Guest 中的 SB 实例使用同一个 lock path 才能实现互斥。

### 0.3 制备包含 actrail-sb daemon 的 Guest 快照

**0.3.1** 在 Guest 模板环境中启动常驻 daemon：

```bash
/usr/bin/actrail-sb daemon --config /etc/actrail/actrail-sb.toml
```

daemon 严格解析静态 TOML配置。

它阻塞`SIGINT`与`SIGTERM`并创建nonblocking `signalfd`，随后准备Guest-local control socket与实例锁的父目录，并取得配置路径上的nonblocking exclusive `flock`。

control socket 或 instance lock path无效、锁冲突或静态配置无效时启动失败。

**0.3.2** daemon 打开配置的 Guest procfs root，读取 Guest boot ID，扫描配置的被观测进程二进制名，并建立启动时已经存在的根与后代关系。

进程名匹配读取 `<pid>/comm`，受 Linux `TASK_COMM_LEN` 限制。

如果 `require_initial_root=true` 且没有找到根，daemon 启动失败。

**0.3.3** daemon 打开内嵌 sandbox BPF object，按配置调整 map容量，load object，并校验 `tracked_processes`、`pending_io`、`io_aggregates`、`oom_events` 和 `collection_diagnostics` map layout。

随后 attach read、write、process fork、process exit 与 OOM victim tracepoints，并把启动扫描发现的根与现有后代 seed 到 `tracked_processes`。

这些 BPF object、maps 和 links 完全由 `actrail-sb daemon` 拥有，与 `actraild` 的 eBPF collector 无关。

**0.3.4** daemon 创建独立 resource reader，读取 boot ID，并完成 CPU、memory 与 OOM 累计计数的初次读取校验。

它同时初始化有界 observation queue、batch encoder、VSOCK session owner 和发送缓冲区。

这些发送设施的初始化不建立 VSOCK socket。

**0.3.5** daemon 创建以下常驻线程：

- `actrail-sb-io`：周期刷新进程根、读取 eBPF I/O aggregate 并排空 OOM victim queue；
- `actrail-sb-resource`：周期读取 Guest CPU、memory 和 OOM 累计计数；
- `actrail-sb-vsock`：拥有连接、握手、发送、Heartbeat 和轻量重连；
- `actrail-sb-control`：非阻塞监听 Guest-local control socket、维护有界connection与deadline；
- `actrail-sb-control-dispatch`：串行执行control command，不阻塞control poll owner。

任一必要 worker 或 control listener创建失败会使 daemon 启动失败。

**0.3.6** daemon 初始化 `publication_enabled=false`，并在所有采集与本地控制设施 ready 后输出 daemon ready。

此时 eBPF 和资源采样器持续工作。

每轮采集结果在 publication boundary立即丢弃，不进入 observation queue，不写文件或数据库，也不形成等待以后补发的 pending batch。

**0.3.7** 沙箱制备方只在 daemon ready 且 `publication_enabled=false` 时制作 Firecracker 快照。

快照中包含已经加载的 eBPF programs、maps、采集线程、发送设施和 Guest-local control listener。

快照中不存在 VSOCK connection、`sb_id` 或运行时 endpoint。

## 1. 启动 actraild

### 1.1 初始化 daemon 核心

**1.1.1** 在 Host 上启动 daemon：

```bash
/usr/bin/actraild --config /etc/actrail/operator.conf run
```

此时尚未启动 gateway，也尚未恢复承载 `actrail-sb daemon` 的运行时 microVM。

因此资源告警插件与 Evidence DB 理论上不会收到任何 sandbox observation。

**1.1.2** `actraild` 主线程严格解析完整 OperatorConfig。
它校验 daemon 核心配置和执行隔离子配置。

**1.1.3** 主线程安装 `SIGINT`/`SIGTERM` handler，写入完整配置指定的 PID file，初始化 Host ID，并构建原有 daemon wiring/runtime。
执行隔离通路复用这个 daemon 进程，但不进入既有 Ingest Pipeline。

### 1.2 初始化 sandbox-resource-alert 插件

**1.2.1** daemon 首先加载 `[plugins.startup]` 中显式配置的插件，然后加载 persistent plugin registry 中的插件。
startup failure policy 控制显式启动项的失败行为（当前 profile 默认 `fail-fast`）。
资源告警插件加载失败会使 daemon 启动失败。
如果某个 startup 项配置为 `continue`，该项失败只会被记录。
persistent registry 中任一插件加载失败仍会使启动失败。

**1.2.2** plugin host 读取 manifest，校验以下内容：plugin ID、API version、builtin runtime、`sandbox-observation-consumer` role、非空且无重复的 selector、consumer queue capacity，以及不需要 host grants。

**1.2.3** manifest selector 决定插件订阅的 observation kind（当前 profile 默认同时订阅）：

- `guest-resource`：Guest CPU、memory、OOM 状态；
- `process-io`：命名根进程谱系的采样区间读写计数。
- `oom-victim`：内核选中的 OOM victim 及其被观测谱系归因。

后续 daemon 收到这三类 observation 时会把它们投递给此插件。
某个 observation 如果被至少一个插件订阅，就不会因为该插件处理失败而回退到 Evidence DB。

**1.2.4** plugin host 读取资源告警插件配置（当前 profile 路径为 `/etc/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.json`），完成 JSON 反序列化和 unknown-field 检查。

**1.2.5** daemon 在加载 startup plugin 前校验独立 Sandbox Alert DB 的绝对路径、schema、queue、transaction、flush、retention、capacity 和线程栈配置。
随后打开 SQLite、校验或创建 schema、推进 ingest epoch、验证只读连接，并启动 database owner（当前 profile 路径为 `/var/lib/actrail/sandbox-alerts.sqlite`）。

**1.2.6** Sandbox Alert DB owner 创建配置容量的有界 queue 和进程内线程 `actrail-sandbox-alert-store`。
线程独占 SQLite write connection，并按 transaction alert limit 与 flush interval 聚合提交。
启动线程必须通过 schema 和 read-capability readiness handshake，失败时 daemon 启动失败。

**1.2.7** database owner ready 后，plugin host 创建 `SandboxResourceAlertPlugin`，校验 CPU、available-memory、read/write 阈值和 source-state capacity。
插件只获得窄化的 nonblocking write port，不知道数据库路径、schema 或 forwarding transport。

四个阈值通过 Web plugin config API 在线修改。
source-state capacity 在插件加载时确定，Web 将其显示为只读字段。

**1.2.8** plugin host 随后创建资源告警 consumer 及其配置容量的有界消费队列（当前 profile 默认 1024），再创建进程内线程 `sandbox-plugin-{consumer_id}`。
`consumer_id` 是运行时分配值，线程名中的数字不是协议常量。

**1.2.9** consumer worker thread创建成功后，registry 立即推进 generation并发布新的不可变消费注册快照。
consumer 没有单独 readiness handshake。
此后 Hand 路由只读取该快照，不在每条 observation 的热路径上调用插件探测接口。

### 1.3 初始化独立 Sandbox Evidence DB

**1.3.1** daemon 校验 `[sandbox_evidence]`，并准备 Evidence DB path 的父目录（当前 profile 路径为 `/var/lib/actrail/sandbox-evidence.sqlite`）。

**1.3.2** daemon 创建配置容量的有界 writer queue（当前 profile 默认 1024 batch）和 readiness channel。
随后创建进程内线程 `actrail-sandbox-evidence`。

**1.3.3** Evidence writer 以 SQLite `READ_WRITE | CREATE | NO_MUTEX` 模式打开独立数据库。
它设置配置的 busy timeout（当前 profile 默认 5 秒）、SQLite synchronous mode（当前 profile 默认 `normal`）和 WAL autocheckpoint pages（当前 profile 默认 1000）。

**1.3.4** writer 初始化或校验配置的 schema version（当前 profile 默认且当前支持版本为 2）。

**1.3.5** writer 从 meta 表读取并持久化推进 `ingest_epoch`。
`ingest_epoch` 用于区分 Evidence store 不同启动周期；
它独立于 `gateway_id`、`sb_id` 和 `ObservationBatch.sequence`。

**1.3.6** writer 打开 read-only probe，验证 schema 和读取能力，统计当前 retained rows，并通过 readiness channel 返回 ready。
daemon 主线程会等待这个结果；
Evidence DB 未 ready 时不会提前开放 Hand TCP listener。

**1.3.7** Evidence ready 后，daemon 使用 plugin matcher/publisher 与 Evidence write port 组装 `SandboxPluginRouteSink`。
该 route sink 只处理 sandbox observation，不连接主 Storage。

### 1.4 启动 Hand TCP listener 并进入 ready

**1.4.1** daemon 在配置的 Hand endpoint bind TCP listener（当前 profile 默认 `127.0.0.1:9472`）。
它将 listener 设置为 nonblocking。
随后按 gateway 连接上限（当前 profile 默认 64）创建 `GatewayIngestRuntime`，并创建进程内线程 `actrail-gateway-accept`。

**1.4.2** `actrail-gateway-accept` 只负责 accept、回收和 join gateway connection worker。
它使用 nonblocking accept；
遇到 `WouldBlock` 或 accept error 时按 accept 轮询周期等待（当前 profile 默认 20 ms），有连接时立即继续处理。
现存 worker 数达到配置的 gateway 连接上限（当前 profile 默认 64）时，新 socket被直接拒绝；
只有通过容量检查的 socket才创建使用配置线程栈（当前 profile 默认 524,288 B）的进程内线程 `actrail-gateway-pending`。
尽管线程名包含 `gateway`，这个线程位于 `actraild` 进程，不在 gateway 进程中。

**1.4.3** 每个 connection worker 设置 `TCP_NODELAY`、配置的 connection read timeout（当前 profile 默认 250 ms）和 write timeout（当前 profile 默认 1 s）。
新连接的第一帧必须是空 payload `GatewayHello`；
仅仅完成 TCP accept 还不表示 gateway 已注册。

**1.4.4** Hand accept thread 成功后，daemon 主线程 bind 完整 operator 配置指定的原有 control UDS，初始化 control connection/drain 状态，执行 ready callback并输出 daemon ready，然后才进入 control UDS serve loop。

**1.4.5** 此时 `actraild` 已 ready，但 gateway 尚未启动：

- resource-alert consumer 正阻塞等待自己的有界队列；
- Sandbox Alert DB writer 正阻塞等待告警；
- Evidence writer 正阻塞等待首个 archive batch；
  首 batch到达后，按 transaction aggregation deadline（当前 profile 默认 250 ms）合并 batch；
  合并数量不超过 transaction batch limit（当前 profile 默认 32）；
  空队列超时不会执行 SQLite flush/checkpoint；
- Hand TCP accept thread 正等待 gateway 连接；
- 没有 sandbox observation 进入任何分支。

## 2. 启动 actrail-vsock-gateway

### 2.1 加载并校验 gateway 配置

**2.1.1** 在 sandbox 所在 Host 启动 gateway。
当前 profile 假设它与 `actraild` 在本机，并使用以下 gateway 配置路径：

```bash
/usr/bin/actrail-vsock-gateway --config /etc/actrail/actrail-vsock-gateway.toml
```

**2.1.2** gateway 主线程严格解析配置并校验 listener backend、backend endpoint、backlog、TCP upstream 地址、容量、周期、I/O timeout 和线程栈。
当前 Firecracker profile使用绝对 `uds_path` 与具体 port形成Host Unix listener（当前默认base path `/run/firecracker/actrail/vsock.sock`、port `43182`、backlog `128`）。

**2.1.3** gateway 校验全局 outbound queue 必须覆盖 `max_sb_connections × per_sb_forward_quota`。
当前 profile 默认关系为 `64 × 16 = 1024`，保证单个 SB 不能占用其他 SB 的保留转发额度。

### 2.2 注册 daemon TCP upstream

**2.2.1** gateway 在创建 VSOCK listener之前先连接配置的 daemon upstream endpoint。
daemon upstream endpoint 与 connect/read/write timeout 均由配置提供（当前 profile 默认 `127.0.0.1:9472` 和 1 秒），并为 TCP stream设置 `TCP_NODELAY`。

**2.2.2** `actrail-gateway-accept` 接受 TCP socket并创建一个 `actrail-gateway-pending` worker。
worker 仍处于 pending 状态，等待第一帧。

**2.2.3** gateway 发送空 payload `GatewayHello`。
daemon worker 校验 frame magic、version、code 和 payload length，然后向 `GatewayIngestRuntime` 预占一个活动连接槽。

**2.2.4** `GatewayIngestRuntime` 为这条 TCP connection 分配非零 `gateway_id`。
该 ID 在 daemon 生命周期内单调推进，`0` 为保留值。
daemon 回复 4-byte big-endian ID 的 `GatewayWelcome`。

**2.2.5** gateway 校验 welcome code 与非零 ID。
初始 TCP connect、Hello/Welcome 或 ID 校验失败时，gateway 直接启动失败；
初始启动不会在后台无限等待 daemon。

**2.2.6** 注册成功后，gateway 创建配置容量的全局 outbound queue（当前 profile 默认 1024），并创建进程内线程 `actrail-gateway-upstream`。
该线程持有已注册 TCP stream，负责 queue→TCP、独立 upstream Heartbeat 和运行期 TCP 重连。

### 2.3 开放 VSOCK listener

**2.3.1** 只有 TCP 注册和 upstream thread 成功后，gateway 才 bind 配置的 VSOCK endpoint。

gateway 随后设置 nonblocking、创建 SessionRegistry，并创建进程内线程 `actrail-gateway-vsock-accept`。

Firecracker profile 监听 `${uds_path}_${port}`。

一个 gateway 实例拥有一个 microVM endpoint。

native AF_VSOCK profile 使用 bind CID 与 port。

StratoVirt 使用该 native profile；通常 bind `VMADDR_CID_ANY`，由 gateway 的 session
registry 和容量约束管理来自 Guest 的连接。

Cloud Hypervisor profile 使用完整绝对 Unix socket path。

三种 backend 共用相同的 SB 连接上限、session registry 与 TCP upstream 行为。

**2.3.2** `actrail-gateway-vsock-accept` 使用 nonblocking accept；
遇到 `WouldBlock` 或 accept error 时按 accept 轮询周期等待（当前 profile 默认 20 ms）再检查停止状态，有连接时立即继续处理。
现存 worker 数达到配置的 SB 连接上限（当前 profile 默认 64）时，新 socket被直接 drop；
只有通过容量检查的 SB socket才创建使用配置线程栈（当前 profile 默认 524,288 B）的进程内线程 `actrail-gateway-sb-pending`。

### 2.4 完成 gateway ready

**2.4.1** gateway ready 的前置条件是：初始 `GatewayWelcome(nonzero)` 曾成功、upstream thread 已创建、VSOCK bind/nonblocking 已成功、SessionRegistry 已创建、VSOCK accept thread 已创建。
此时还不要求任何 SB 接入。

**2.4.2** upstream thread 可能在 ready 输出前发现初始 TCP 已断开，并把当前 `gateway_id` snapshot 暂时置为 `0` 后进入重连。
因此 ready 行中的 snapshot 可能为 `0`，但这不改变“初始注册曾成功”的启动条件。

## 3. 恢复 Guest 并连接 actrail-sb daemon

### 3.1 恢复包含常驻 daemon 的 microVM 快照

**3.1.1** 沙箱管理器使用运行时选择的 Firecracker VSOCK base `uds_path` 和 Guest CID恢复 microVM。

Host 上的 gateway 已监听与本次运行时 port 对应的 `${uds_path}_${port}` endpoint。

**3.1.2** 恢复后，`actrail-sb daemon` 延续快照中的 eBPF links、maps、resource reader、采集线程、发送设施、实例锁和 Guest-local control listener。

daemon 不恢复 VSOCK connection，也没有运行时 Host CID、port 或 `sb_id`。

`publication_enabled` 保持 `false`。

**3.1.3** StratoVirt/Kata 不依赖 Firecracker snapshot restore。Guest 正常启动后运行
同一个 `actrail-sb daemon`，等待其在没有 VSOCK session 的状态下本地 ready，再进入
下述 CLI connect 流程。daemon、control、generation gate 和 Session Owner 语义不变。

### 3.2 通过 actrail-sb CLI 下发运行时 endpoint

**3.2.1** 沙箱管理器在 Guest 内执行短生命周期 CLI：

```bash
/usr/bin/actrail-sb connect \
  --control-socket <guest-control-socket> \
  --host-cid <runtime-host-cid> \
  --port <runtime-vsock-port>
```

Host CID、port 和 control socket path均为概念参数。

当前 profile 中，Host CID、VSOCK port 和 control socket path 的默认值分别为（`2`）、（`43182`）和（`/run/actrail/actrail-sb-control.sock`）。

**3.2.2** CLI 连接 Guest-local control socket，并发送有界的 connect request。

CLI 不加载 eBPF，不读取 procfs，不创建采集线程，也不直接持有 VSOCK session。

control socket 不经过 `actrailctl`、gateway 或 `actraild`。

**3.2.3** Control Client使用配置的binary frame上限编码一个Connect command，写完后关闭socket写方向并等待一个response。

CLI request timeout只限制CLI等待响应，不撤销daemon已经接受的Connect。

**3.2.4** `actrail-sb-control` poll owner接受socket后创建单命令connection owner。

connection owner预分配受frame limit约束的buffer，并持有从接受时刻开始计算的connection deadline。

poll owner同时监听listener、stop fd、dispatcher wake fd和全部accepted connection；它不执行VSOCK connect或handshake。

**3.2.5** 读取到完整合法命令后，poll owner尝试把命令非阻塞交给单槽有界、单worker dispatcher。

dispatcher空闲时立即接收并调用`SandboxControlPort`。

已有control command正在执行时，第二个命令不排队，立即得到`Rejected(Busy)`。

accepted connection数量达到配置上限（当前profile默认8）时，poll owner停止从listener继续accept，已有connection完成或过期后再恢复admission。

**3.2.6** runtime control owner为被接受的命令建立独立completion state，并使用daemon配置的control timeout限制服务端占用时间。

CLI等待超时不撤销已接受的Connect。

处理中再次调用立即返回`Busy`；操作完成后，相同端点重试幂等返回当前session。

**3.2.7** daemon 校验 Host CID、port 和请求形状。

Host CID与port不能使用相应的 `ANY` 保留值。

请求无效时只拒绝当前 CLI 请求。

常驻 daemon 与采集设施继续运行。

daemon 不改变当前 runtime endpoint、VSOCK session 或 `publication_enabled`。

### 3.3 建立 VSOCK session

**3.3.1** `actrail-sb-vsock` 使用 CLI 下发的 endpoint 执行受sender I/O timeout限制的nonblocking AF_VSOCK connect，并设置运行期read/write timeout。

Firecracker 将 Guest 的 AF_VSOCK connect转换为 Host `${uds_path}_${port}` 上的 AF_UNIX connect。

StratoVirt 则通过 vhost-vsock 把同一 Guest AF_VSOCK connect交给 Host native
AF_VSOCK listener；该差异不进入 Session Owner、gateway session 或 upstream runtime。

**3.3.2** gateway accept连接后创建 `actrail-gateway-sb-pending` worker。

worker必须在配置的 `sb_peer_idle_timeout` 内收到合法空 `SbHello`。

随后它预占 SessionRegistry活动槽，分配非零 `sb_id`，创建配置的 per-SB quota，并回复 `SbWelcome(sb_id)`。

**3.3.3** daemon 只有在 AF_VSOCK connect、`SbHello/SbWelcome` 和非零 `sb_id` 校验全部成功后，才认为连接建立。

任一步失败时，daemon 关闭当前 socket并保持 `publication_enabled=false`。

CLI 收到失败结果后以非零状态退出。

daemon 不退出，采集器不卸载。

### 3.4 建立新连接的发送边界

**3.4.1** 握手成功后，daemon 在 publication仍关闭时请求I/O worker读取并丢弃当前eBPF aggregate，建立显式进程I/O baseline。

baseline cycle只要包含任一collector failure，本次Connect就失败，gate保持关闭。

baseline成功后，Session Owner丢弃旧发送队列与pending batch。

断连期间累积的读写计数不会进入新连接的第一条 `ProcessIoCounters`。

**3.4.2** daemon 为新 session设置从 1 开始的 batch sequence和当前非零 `sb_id`，然后设置 `publication_enabled=true`。

该写入是采集结果进入发送队列的唯一开关。

**3.4.3** 只有control request仍允许commit时，daemon才发布active session并通过dispatcher把连接成功响应交还control poll owner。

poll owner写出有界response frame并关闭该connection。

CLI 随后退出。

常驻 `actrail-sb daemon` 继续持有采集设施与 VSOCK session。

## 4. Guest 日常采集与即时发送

### 4.1 无被观测进程时持续采集资源

**4.1.1** root refresh 按配置周期扫描被观测进程的二进制名（当前 profile 默认周期 1 秒，二进制名 `xiaoo`、`claude`）。
没有命名根时，I/O poll 不产生 `ProcessIoCounters`，但 `actrail-sb-io` 线程继续运行，不把“当前无 root”当成运行期致命错误。

**4.1.2** `actrail-sb-resource` 与被观测进程是否存在无关。
它按 resource poll interval（当前 profile 默认 1 秒）读取配置的 procfs root（当前 profile 默认 `/proc`）下的 `stat`、`meminfo` 和 `vmstat`，形成 `GuestResourceSnapshot`：

```text
guest_boot_id
sampled_at_ms
cpu.total_ticks / idle_ticks / logical_cpu_count
memory.total_bytes / available_bytes / used_bytes / oom_kill_count
```

**4.1.3** resource thread 完成采样后先读取 publication gate。

`publication_enabled=true` 时，它使用 nonblocking `try_send` 把快照放入当前 VSOCK session的 observation queue。

`publication_enabled=false` 时，它立即丢弃快照。

丢弃的快照不进入发送队列，不写入 Guest 文件或数据库，也不在以后补发。

**4.1.4** 连接建立后，因为 resource poll interval（当前 profile 默认 1 秒）小于 max silence interval（当前 profile 默认 5 秒），所以健康运行时通常每个 resource poll interval 都会产生一个 ObservationBatch，SB Heartbeat 理论上不会触发。

未连接时不发送 Heartbeat。

**4.1.5** resource-alert 是否订阅 `guest-resource` 由 manifest selector 决定（当前 profile 默认订阅），所以这些快照会被插件消费，而不是进入 NoInterest Evidence DB。
如果 available memory 已低于配置的告警阈值（当前 profile 默认 512 MiB），则可以触发一次从非风险状态进入风险状态的 `OomRisk`。

### 4.2 发现 bash 拉起的命名根和后代

**4.2.1** 假设用户执行：

```bash
bash -lc '/usr/local/bin/xiaoo ...'
```

该命令以当前 profile 的被观测进程二进制名 `xiaoo` 为例。
`bash` 本身不属于目标 lineage，因为它不在当前 profile 的 `root_process_names` 中（当前值为 `xiaoo`、`claude`）。
匹配依据是 bash 启动出的进程执行目标程序后，在配置的 procfs root 下读取到的 `<pid>/comm`，不是 shell 命令文本；
shell 既可能先 fork子进程，也可能直接在当前 PID 上 exec最后一个命令。

**4.2.2** `exec -a` 只修改 `argv[0]`，不能保证 `comm` 变成目标名称。
目标必须具有相符的真实 executable name，或者包装器显式使用 `prctl(PR_SET_NAME, ...)` 并把该名称加入 `root_process_names`。

**4.2.3** 如果被观测的命名根在 SB 启动后出现，它最多等待一个 root refresh interval 才被发现（当前 profile 默认 1 秒）。
seed 发生前的早期 read/write 不会被追溯；
seed 之后的 I/O 才开始累计。

**4.2.4** root refresh 读取 PID、start time、comm 和父 PID，确认新命名根是独立根，然后写入 `tracked_processes[root_tgid] = root_marker`。

**4.2.5** 该命名根后续通过 `fork`、`vfork` 或 `clone` 创建后代时，Guest kernel 的 `sched_process_fork` BPF program检查父 TGID。
如果父 TGID 已被跟踪，它把同一个 root marker复制给 child PID。

**4.2.6** 子进程随后 `exec` 其他 agent、shell 或 tool 时不会改变已经记录的 lineage 归属。
这个后代的 read/write 仍聚合到最初命名根的 marker。

**4.2.7** 如果后代执行后自己的 `comm` 也匹配另一个配置根名，refresh 不会覆盖已经存在的 tracked lineage；
它不会被错误提升为新的根。
启动扫描时已经存在的嵌套命名根则会预先分配为彼此独立的 roots。

### 4.3 在 eBPF 热路径累计 read/write

**4.3.1** 在 `sys_enter_read` 或 `sys_enter_write`，BPF 取得完整 `pid_tgid`，以 TGID 查询 `tracked_processes`。
未命中时立即返回，不复制用户缓冲区、不发用户态事件、不计算哈希。

**4.3.2** 命中时，BPF 把 `{root marker, read/write kind}` 写入 `pending_io[pid_tgid]`。
使用完整 `pid_tgid` 可以区分同一进程的不同线程。

**4.3.3** 在对应的 `sys_exit_read` 或 `sys_exit_write`，BPF 读取并删除 pending entry，再按 root marker查找或创建 `io_aggregates`。

**4.3.4** syscall 返回值小于 0 时只增加 `failed_read_operations` 或 `failed_write_operations`；
返回值大于等于 0 时增加成功 operation，并把实际返回字节数加入 `read_bytes` 或 `write_bytes`。
返回 0 仍算一次成功 operation，但增加 0 bytes。

**4.3.5** 当前基础采集只覆盖 `read` 和 `write` syscall，不覆盖 `pread`、`pwrite`、`readv`、`writev` 等其他 I/O syscall。

**4.3.6** `sched_process_exit` 清理退出线程的 pending entry 和 tracked entry。
某个 root 已无活跃 tracked PID 时，用户态读取最后一个 delta 后回收对应 aggregate 与 baseline。

### 4.4 轮询 I/O 与资源并经过发送门控

**4.4.1** `actrail-sb-io` 按 I/O poll interval 先执行 root refresh（当前 profile 默认 1 秒）。
refresh 失败仍继续读取已有 lineage aggregates；
aggregate 或采样时钟读取失败时，本轮不产生 I/O observation。
只有显式启用 diagnostics 时，这些 failure才累计到 collector diagnostics；
diagnostics 是否启用由配置控制（当前 profile 默认关闭）。
关闭时，这些失败按 fail-local语义静默结束本轮。

**4.4.2** 用户态为每个 root 保存上次 aggregate baseline，并对本轮累计值计算 saturating delta。
全零 delta 不产生 observation；
存在增量时形成一条 `ProcessIoCounters`，包含：

```text
guest_boot_id
root pid / start_time_ticks / executable_name
sample_started_ms / sample_ended_ms
read_operations / read_bytes
write_operations / write_bytes
failed_read_operations / failed_write_operations
```

**4.4.3** `actrail-sb-resource` 按独立的 resource poll interval（当前 profile 默认 1 秒）生成 `GuestResourceSnapshot`。
资源读取失败只跳过本轮，不停止 I/O collector；
I/O 读取失败也不停止资源采样。

**4.4.4** 两个生产线程在每条 observation 的 publication boundary读取连接门控：

- Disabled：立即丢弃 observation，不进入 queue；
- Enabled：对当前 session的配置容量 queue（当前 profile 默认 1024）调用 `try_send`。

queue admission结果为：

- Accepted：立即返回，不等待 sender；
- Full：丢弃当前 observation，不阻塞 collector；
- Closed：丢弃当前 observation，并在 publication gate切换后继续下一轮采集。

transport session断开不等于 producer channel永久关闭。

运行期 VSOCK 故障只关闭当前 session queue，并把 publication gate切回 Disabled；采集线程继续下一轮。

**4.4.5** diagnostics 是否启用由配置控制（当前 profile 默认关闭）。
关闭时，成功路径不会为了周期状态输出执行额外 atomic metrics 更新、字符串格式化或 stdout/stderr I/O。

### 4.5 即时组成 batch 并执行最大静默 Heartbeat

**4.5.1** `publication_enabled=true` 时，`actrail-sb-vsock` 在当前 session queue为空时按 max silence interval 设置 `recv_timeout`（当前 profile 默认 5 秒），等待第一条 observation。

第一条到达后立即被唤醒；

sender 随即用 nonblocking `try_recv` 获取当时已经就绪的其他项，最多合并 batch observation limit 指定的条数（当前 profile 默认 256）。

`publication_enabled=false` 时，sender不消费 observation，不构造 batch，也不运行 Heartbeat计时。

**4.5.2** sender 不等待凑满 batch。
低流量时通常一条资源快照立即形成一个 batch；
积压时连续发送多个有界 batch，目标是尽快把队列清空。

**4.5.3** sender 构造 `ObservationBatch(sequence, observations)`。

sequence 从 1 开始；

只有完整 frame `write_all` 成功后才清空 pending、递增 sequence并刷新 `last_frame_write`。

pending 只属于当前已连接 session。

连接失败时立即丢弃，不跨连接保存或重发。

**4.5.4** SB frame 使用固定 8-byte header：magic、version、message code 和 big-endian `u32 payload_length`；
最大 frame 为 256 KiB。
payload 使用紧凑二进制 codec，不使用 JSON。

**4.5.5** 只有队列持续为空，并且距最后一次成功写任意 SB frame 已达到配置的 `max_silence_interval`（当前 profile 默认 5 秒）时，sender 才发送空 payload Heartbeat。
Heartbeat 不包含 observation；
gateway 只用它刷新 SB session activity，不把它上送 daemon。

## 5. gateway 代理 VSOCK 并发送 TCP upstream

### 5.1 处理 SB VSOCK frame

**5.1.1** 每个 `actrail-gateway-sb-pending` worker 为自己的 VSOCK session维护非零 `sb_id`、最近活动时间和 quota。
读取到任意合法 SB frame后刷新 `last_activity`。

**5.1.2** 收到空 Heartbeat 时，worker 只刷新 activity并继续读取。

### 5.2 封装并进入有界 upstream queue

**5.2.1** 收到 `ObservationBatch` 时，gateway 不解码 observation payload，不执行进程关联、插件匹配或落盘。
它保留完整原始 SB frame，构造：

```text
ForwardedSbFrame payload
├── 4-byte big-endian sb_id
└── complete original SB frame
```

**5.2.2** gateway 再为 `ForwardedSbFrame` 添加 upstream 固定 header，先从当前 SB 的 quota 预留一个 permit，再 nonblocking `try_send` 到配置容量的全局 outbound queue（当前 profile 默认 1024）。

**5.2.3** 配置的 per-SB quota 已满（当前 profile 默认 16）、全局 queue 已满或 upstream channel关闭时，当前 SB worker结束并关闭该 VSOCK session。
当前尚未入队的 batch会丢失；
该 SB 已入队但尚未发送的 frames 也可能因 session quota失活而被跳过。
其他 SB session 不受影响。

### 5.3 关闭失活或故障的 SB session

**5.3.1** 达到配置的 SB peer idle timeout 仍未收到任何 SB frame（当前 profile 默认 15 秒）、VSOCK EOF、读错误或协议错误时，只关闭当前 SB session并释放活动槽。

### 5.4 发送 TCP upstream 并重连

**5.4.1** `actrail-gateway-upstream` 按全局 queue 顺序取出 ForwardItem并对已注册 TCP stream 执行 `write_all`。
item 完成或被丢弃时释放对应 per-SB permit。

**5.4.2** upstream queue 暂时为空，并且距离上次 upstream Heartbeat 或重连达到配置的 heartbeat interval（当前 profile 默认 5 秒）时，gateway 发送独立空 Heartbeat。
这个计时器不会因为普通 ForwardedSbFrame 写成功而刷新，因此它与 SB 的 observation 最大静默 Heartbeat 不是同一语义。

**5.4.3** ForwardItem 的 TCP `write_all` 失败时，gateway 保留当前 item，把当前 `gateway_id` snapshot 置为 `0`，执行一次 TCP connect与 `GatewayHello/Welcome`；
一次尝试失败后按 reconnect interval 等待再尝试（当前 profile 默认 1 秒）。
取得新的 `gateway_id` 后重发这个明确写失败的 item。

**5.4.4** 独立 upstream Heartbeat 写失败时没有 ForwardItem需要保留，只把 `gateway_id` snapshot置为 `0` 并进入相同重连流程。

**5.4.5** upstream 没有应用层 ACK。
`write_all` 返回成功只表示本地 TCP stack接受了完整 bytes；
如果连接随后丢失而 daemon尚未完整读取和路由该 frame，gateway不会重发，因此存在丢失边界。
只有 `write_all` 明确失败的当前 ForwardItem会在重连后重试。

## 6. actraild 路由、持久化与告警

### 6.1 接收并解码 gateway frame

**6.1.1** `actrail-gateway-pending` worker 的第一帧完成注册后，后续每次 TCP read 大于 0 都刷新 connection activity。
达到配置的 gateway connection idle timeout 仍没有任何字节（当前 profile 默认 15 秒）时只关闭当前 TCP connection。

**6.1.2** 收到 upstream Heartbeat 时只刷新/统计连接活性，不进入 sandbox route sink。

**6.1.3** 收到 `ForwardedSbFrame` 时，worker 校验非零 `sb_id`，恢复完整内层 SB frame，并要求内层恰好包含一个合法 `ObservationBatch`。
协议或解码错误只关闭当前 gateway TCP connection。

**6.1.4** worker 以当前 connection 的 `gateway_id`、frame 内的 `sb_id` 和 batch sequence组成 Hand 来源上下文，然后调用 route sink。
`(gateway_id, sb_id)` 只在这条独立通路中区分来源，不转换为脑侧 identity 或 trace membership。

**6.1.5** route sink delivery失败时，worker记录失败并丢弃当前 batch，但继续读取同一 TCP connection。
下游插件或 Evidence 故障不会反向关闭正常 gateway connection。

### 6.2 按 observation 粒度路由

**6.2.1** route sink 在同一不可变 plugin registry generation 中，为 batch 每条 observation 生成 `(kind, index)`，查询所有订阅该 kind 的 consumers。
每条 observation 可以匹配 0 到 N 个插件。

**6.2.2** 整批都没有订阅者时，route sink 把全体 indices 组成 NoInterest archive batch，并对独立 Evidence queue调用 `try_append`。

**6.2.3** mixed batch 中，route sink 对每个匹配插件只发布一次 `ConsumerBatch`。
该 ConsumerBatch共享原始 observations，并携带此插件匹配的 indices；
同一 observation 会投递给所有匹配插件。

**6.2.4** 同批所有零匹配 observation 另外组成 Evidence archive batch。
匹配 observation 只投插件，未匹配 observation 才写 Evidence。

**6.2.5** 插件 queue Full/Closed 只使对应 consumer admission失败；
其他插件发布与 unmatched Evidence admission仍独立执行。
匹配 observation 不因插件失败回退 Evidence。

**6.2.6** matcher query错误、registry generation 在 match 与 publish 之间变化形成 `ExpiredPlan`、或 publish plan结构校验失败时，route sink直接返回 delivery error。
daemon worker记录并丢弃当前 batch；
此时 unmatched Evidence分支不会执行，也不存在 fallback。
该分支不同于插件 queue Full/Closed 返回的普通 publish report。

**6.2.7** Evidence `try_append` 不在 gateway connection worker 中执行 SQLite I/O：

- TooLarge、Full、Closed：同步拒绝当前 archive batch；
- Accepted：只表示进入 writer queue；
- SQLite transaction 后续失败：记录 `failed_batches` 和 `last_error`，该 transaction 已接纳内容不重试，writer继续处理后续工作。

Evidence 失败不改投插件或主 Storage。

### 6.3 生成资源告警

**6.3.1** consumer 以 `(gateway_id, sb_id)` 作为 source key维护受 source-state capacity 限制的有界状态（当前 profile 默认 4096）。
容量满时淘汰最旧 source state。

**6.3.2** `OomKilled`：`oom-victim` observation 到达时产生。
告警记录 victim PID、victim `comm`、`monitored`、`unmonitored` 或 `unknown` 归因，以及 `monitored` 时的谱系根标记。
所有 OOM 告警均为 critical。
resource snapshot 中的 `oom_kill_count` 作为累计资源指标保留，不产生 OOM 告警。

**6.3.3** `OomRisk`：`available_bytes` 低于 available-memory threshold（当前 profile 默认 536,870,912 B），且 source 从非风险状态进入风险状态时产生。
持续低内存不重复告警；
恢复到阈值以上后再次跌破会再次告警。

**6.3.4** `HighCpu`：相邻同一 Guest boot 的累计 CPU tick 计算出区间利用率，并在利用率从阈值以下进入阈值以上时产生。
首个快照、Guest boot 变化、计数倒退或无有效总 tick 增量时只建立新 baseline。

**6.3.5** `HighRead`：单条 `ProcessIoCounters.read_bytes` 严格大于 per-interval read threshold 时产生（当前 profile 默认 268,435,456 B）。

**6.3.6** `HighWrite`：单条 `ProcessIoCounters.write_bytes` 严格大于 per-interval write threshold 时产生（当前 profile 默认 268,435,456 B）。

**6.3.7** 同一 resource observation 可以同时产生 `OomRisk` 和 `HighCpu`；
同一 I/O observation 可以同时产生 `HighRead` 和 `HighWrite`。

**6.3.8** consumer 对 Sandbox Alert DB writer queue 执行 nonblocking `try_send`。
Full/Closed 时丢弃当前告警并更新告警支路状态，不向 Evidence fallback，也不使 Hand observation 消费失败。

**6.3.9** writer 使用独立 SQLite connection 和有界批量事务写入 Sandbox Alert DB。
事务提交成功后，才把标准化外发副本交给 builtin forwarding plugin。
外发 disabled、category 不匹配、queue 满或 proxy 断开不改变数据库记录。

### 6.4 在线修改资源告警阈值

**6.4.1** Web 读取活动 `sandbox-resource-alert` 实例的当前 JSON 配置和 schema。

**6.4.2** Web 提交候选配置进行校验。
daemon 校验 JSON 形状、四个阈值的数值范围，并拒绝在线修改 source-state capacity。

**6.4.3** 更新请求通过校验后，daemon 在 control 线程将候选配置写入同目录临时文件，完成文件同步并原子替换实例原配置文件。
写入或替换失败时返回错误，活动插件继续使用旧配置。

**6.4.4** 持久化成功后，插件原子发布完整的不可变配置快照。
consumer 在下一批 observation 开始时读取一次该快照。
更新不卸载插件、不更换 consumer，也不清空 CPU baseline 或内存越阈状态。

## 7. 运行期故障与重连

### 7.1 处理 SB VSOCK 故障

**7.1.1** SB 的 ObservationBatch或 Heartbeat写失败时，`actrail-sb-vsock` 先设置 `publication_enabled=false`，再关闭当前 VSOCK socket并清除当前 `sb_id`。

它丢弃当前 pending batch和旧 session queue中的全部 observations。

任何旧 session数据都不进入下一条连接。

**7.1.2** SB 握手完成后不持续读取 VSOCK；

gateway 主动关闭 session通常会在 SB 下一次写入时被发现。

**7.1.3** daemon 保存最近一次已经成功建立 session 的 runtime endpoint，并使用它执行轻量运行期重连。

一次 connect或 handshake失败后，按配置的 reconnect interval等待再尝试。

重连期间 `publication_enabled` 保持 `false`，所有采集结果立即丢弃。

**7.1.4** 重连完成后，daemon 按新连接流程清空发送边界，读取并丢弃当前 eBPF aggregate，重建 I/O baseline，重置 batch sequence，并在获得新非零 `sb_id` 后设置 `publication_enabled=true`。

断连期间的数据不存储、不补发。

**7.1.5** 某 batch 已成功写入 VSOCK、但 gateway 随后无法进入 upstream queue时，SB 无法感知这次下游丢失，也不会重发。

**7.1.6** Guest-local Control Server poll owner异常退出时，它持有的health writer关闭。

daemon main的`ppoll`从health fd收到终止事件，收割server结果并通过统一输出边界记录一次不可用诊断。

main随后移除该health fd并继续事件等待。

I/O worker、resource worker、VSOCK Session Owner和当前data connection保持运行。

新的`actrail-sb connect`无法完成，直到Guest重新启动daemon。

### 7.2 处理 gateway TCP upstream 故障

**7.2.1** gateway ForwardItem写失败时保留当前 item并重连；
Heartbeat写失败只触发重连。
已经 `write_all` 成功、但 daemon尚未读取/路由就断线的 frame不会自动重发；
新 connection获得新的 `gateway_id`。

### 7.3 隔离 daemon 下游故障

**7.3.1** daemon protocol/frame 错误只关闭当前 gateway connection；
不会影响其他 gateway connection。

**7.3.2** sink delivery失败只丢当前 batch并继续同一连接。
matcher/publish plan结构性错误还会跳过该批 unmatched Evidence分支。

**7.3.3** plugin admission 或 consume 错误只影响对应插件。

**7.3.4** Sandbox Alert DB queue、transaction 或 post-commit forwarding 错误只影响告警支路；
不关闭 Hand connection，不向 Evidence 或主 Storage fallback。

**7.3.5** Evidence admission/transaction错误只影响对应 archive work。

## 8. 沙箱生命周期结束

### 8.1 确定停止顺序

**8.1.1** 沙箱结束时先终止 Guest 内的 `actrail-sb daemon`，再回收该 microVM对应的 gateway，最后按 Host 生命周期停止 `actraild`。

该顺序不要求 SB 排空未发送的采集结果。

### 8.2 停止 actrail-sb

**8.2.1** `SIGTERM`/`SIGINT`进入main持有的`signalfd`。

阻塞在`ppoll`上的main被事件唤醒。

main先向Control Server发送stop，停止listener admission和control poll owner。

随后main调用Sandbox Agent Runtime shutdown，设置`publication_enabled=false`并关闭当前VSOCK session。

**8.2.2** sender立即丢弃 pending batch和 observation queue中的未发送数据。

关闭流程不排空采集结果，不为尾部数据建立新连接，也不执行重连。

**8.2.3** I/O worker退出时 drop独立 collector，BPF links/maps随 owner释放。

resource worker与VSOCK worker退出。

agent workers join完成后，main最后join已经停止admission的control poll owner。

**8.2.4** workers join完成后，`SandboxAgentDaemonProcess` owner离开作用域并 drop `_instance_lock`。

此时才释放 lock file上的 `flock`，并删除 Guest-local control socket runtime file。

### 8.3 停止 gateway

**8.3.1** gateway停止时先让 VSOCK accept和活动 workers观察 stop并退出，关闭 session使对应 queued permits失活，然后停止 TCP upstream。

### 8.4 停止 actraild

**8.4.1** daemon 在配置的 supervision budget 下停止（当前 profile 默认 `shutdown_wait_ms=170100`）。
内部顺序为：

1. 停止 Hand TCP accept 和 connection workers；
2. 在 Evidence 自身配置的 drain budget（当前 profile 默认 10 秒）内排空允许排空的 queue，并关闭独立 DB；
3. 关闭 daemon services 和 plugin consumers，停止新的 Sandbox 告警提交；
4. 在 Sandbox Alert DB 自身配置的 drain budget内排空已接纳告警并关闭数据库；
5. 删除 control UDS runtime file 和 PID file。

## 9. 附录：进程与线程归属

### 9.1 actraild 进程

- 既有 main/control UDS serve loop；
- `actrail-sandbox-alert-writer` × 每个 resource-alert instance；
- `sandbox-plugin-{consumer_id}` × 每个 sandbox plugin consumer；
- `actrail-sandbox-evidence` × 1；
- `actrail-gateway-accept` × 1，负责 Hand TCP accept；
- `actrail-gateway-pending` × 每个通过容量检查的 active gateway TCP socket，受配置的连接上限约束（当前 profile 默认 64）。

### 9.2 actrail-vsock-gateway 进程

- main/signal wait；
- `actrail-gateway-upstream` × 1；
- `actrail-gateway-vsock-accept` × 1；
- `actrail-gateway-sb-pending` × 每个通过容量检查的 active VSOCK socket，受配置的连接上限约束（当前 profile 默认 64）。

### 9.3 actrail-sb daemon 与 CLI

- `actrail-sb daemon` 是 Guest 内唯一常驻 SB 进程；
- daemon main/signal wait及可选低频 diagnostics；
- main通过`signalfd + ppoll`等待shutdown signal、control health fd和可选diagnostics deadline；diagnostics关闭时无周期唤醒；
- `actrail-sb-io` × 1；
- `actrail-sb-resource` × 1；
- `actrail-sb-vsock` × 1；
- `actrail-sb-control` × 1，非阻塞poll owner；
- `actrail-sb-control-dispatch` × 1，单槽有界、非阻塞admission的单command dispatcher；
- 每次 `actrail-sb connect` 是短生命周期 CLI 进程，只连接 Guest-local control socket，等待 daemon返回连接结果后退出。

### 9.4 Guest kernel

- 4 个 read/write syscall tracepoint programs；
- 1 个 process fork tracepoint program；
- 1 个 process exit tracepoint program；
- 4 个 BPF maps。

这些 kernel objects由 `actrail-sb` 生命周期拥有，不属于 `actraild`。
