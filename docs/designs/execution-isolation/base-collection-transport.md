# 执行隔离采集与观测通路设计

## 1. 系统边界

本设计定义一条独立于脑侧观测链路的 Hand 数据通路：

```text
Guest Agent / workload
    ↓ Guest 内独立观测
actrail-sb
    ↓ AF_VSOCK
Firecracker VMM
    ↓ Host AF_UNIX
actrail-vsock-gateway
    ↓ TCP
actraild / GatewayIngestRuntime
    ├── 有插件声明消费意向 → 对应插件
    └── 无插件声明消费意向 → Sandbox Evidence DB
```

该通路不经过 `actrailctl`，不复用 `actraild` 的 eBPF 程序、map、collector 或进程跟踪状态。`actrail-sb` 自带的短生命周期 CLI 只控制 Guest 内 daemon 的 VSOCK 连接，不进入 observation 数据通路。Hand 原始 observation 不进入现有 Ingest、Identity、Trace、Semantic、Recording、Export 和主 Storage 链路；匹配插件从 observation 派生出的告警属于插件输出，不改变原始 observation 的路由边界。

通路只承载以下 Guest-local observation：

- 从配置的目标进程名集合识别谱系根，并按根进程聚合该根及其后代进程的读写操作次数和字节数。
- 轮询 Guest 当前 CPU、内存和 OOM 状态并生成资源快照。

数据模型不包含文件内容、网络内容、系统调用 payload、脑侧进程身份、trace id 或 semantic action。

连接基数为一个 `actraild` 接受多个 gateway。

Firecracker 主线中，一个 gateway 实例拥有一个 microVM endpoint。

每个 Guest kernel 只允许一个 `actrail-sb` 实例。

连接上限约束该 endpoint 上的并发与重连连接。

连接上限不表示 gateway 自动发现其他 microVM 路径。

## 2. 容器职责

### 2.1 `actrail-sb`

`actrail-sb daemon` 是运行在 Guest 内的单实例常驻采集进程。一个 Guest kernel 最多运行一个 daemon；进程通过 Guest-local 实例锁在启动时执行互斥检查。

daemon 在制作 Firecracker 快照前完成 eBPF load/attach、maps 创建、资源采样器、采集线程、发送设施和 Guest-local control listener 初始化。daemon ready 不依赖 VSOCK 连接，快照中不包含有效的 VSOCK session。

快照恢复后，Guest 内执行短生命周期命令：

```text
actrail-sb connect \
    --control-socket <guest-local-path> \
    --host-cid <runtime-host-cid> \
    --port <runtime-port>
```

CLI 通过 Guest-local control socket 把本次运行时 endpoint 交给 daemon。daemon 完成 AF_VSOCK connect 和 `SbHello/SbWelcome` 后，CLI 才返回连接成功。CLI 不加载 eBPF、不启动采集线程，也不持有 VSOCK 数据连接。

Guest-local control listener由一个非阻塞poll owner维护多个有界connection。

完整 Connect命令交给独立单worker dispatcher，因此VSOCK connect、handshake和baseline不会阻塞listener接收其他connection。

dispatcher不保存等待队列。

已有命令正在执行时，另一个合法Connect命令立即返回Busy。

CLI的请求时限约束外部等待；同一时限传入daemon control owner，过期或已取消请求不能在CLI放弃后提交新session。

daemon 内部包含五项职责：

```text
目标进程谱系与 I/O 计数采集
Guest 资源状态轮询
Guest-local CLI 控制
有界 observation 聚合
VSOCK 会话与发送
```

#### 目标进程谱系与 I/O 计数采集

配置提供一组目标进程名，按 Linux `comm` 精确匹配。所有匹配实例分别成为谱系根，采集器将根进程及其通过 `fork`、`vfork` 或 `clone` 创建的后代纳入观测范围。后代执行 `exec` 后仍属于原谱系；进程退出后从活跃集合移除。每个谱系使用根进程在 Guest boot 范围内的 PID、启动时间和根发现时匹配的 `comm` 作为稳定标记，避免 PID 复用造成错误归属。

I/O 计数由 `actrail-sb` 自己装载的 Guest-side eBPF 程序采集，包括：

- 成功完成的 `read(2)` 次数和实际读取字节数。
- 成功完成的 `write(2)` 次数和实际写入字节数。
- 返回负值的 `read(2)` 和 `write(2)` 次数。

内核侧将同一谱系内所有成员的计数聚合到根进程标记，用户态按配置周期读取各根谱系的增量。失败调用只增加对应失败次数，不增加成功次数和成功字节数。eBPF 热路径不复制用户缓冲区、不计算内容哈希、不逐次向用户态发送系统调用事件。

Guest-side eBPF 对象、map、attach point、用户态 reader 和生命周期均由 `actrail-sb` 独立拥有，与 `actraild` 的 eBPF 采集完全无关。

两个collector只共享轻量的标准tracepoint挂载策略：按section区分标准tracepoint与其他program；标准tracepoint通过`perf_event_open`并强制使用`PERF_EVENT_IOC_SET_BPF`挂载，其他program继续使用libbpf原生attach。该共享层不共享BPF对象、map、采集状态或生命周期。`actrail-sb`缺失或无法挂载任一必要tracepoint时启动失败。

#### Guest 资源状态轮询

资源采样器独立于 eBPF 采集器，按配置周期读取 Guest `procfs_root`；checked-in default为 `/proc`。boot标记来自该根下的 `sys/kernel/random/boot_id`，CPU来自 `stat`，内存来自 `meminfo`，OOM计数来自 `vmstat` 的 `oom_kill`。每个快照包含：

- Guest CPU 累计总时间、空闲时间和逻辑 CPU 数量。
- Guest 内存总量、可用量和已用量。
- Guest `oom_kill` 单调计数。
- Guest boot 标记和采样时间。

资源快照描述整个 Guest 环境，不绑定某个进程谱系，也不要求目标进程处于运行状态。

#### Observation 聚合与发送

daemon 维护一个由当前 VSOCK session 状态驱动的发送门控。只有 `SbHello/SbWelcome` 完整成功且当前 session 仍有效时，门控才允许 observation 进入有界发送队列。

未连接、连接中或已经断开时，eBPF 采集和资源轮询继续运行，但本轮产生的 observation 在发送边界立即丢弃，不进入发送队列、不写本地文件或数据库，也不等待后续连接补发。I/O 轮询仍推进累计计数基线，避免未连接期间的读写量进入下一条连接。

连接有效时，进程 I/O 增量和资源快照进入同一个有界 observation 队列。发送线程收到第一条 observation 后立即发送，并将当时已经就绪的 observation 合并为同一个有界 batch，不等待队列凑满；持续积压时连续发送多个 batch。协议自身的最大 frame 长度同时约束可配置的 batch 数量。发送端变慢不得阻塞 eBPF 热路径或资源轮询；队列容量耗尽时记录显式丢弃计数，不建立无界缓存。

检测到 ObservationBatch 或 Heartbeat 写失败，或者本地 session 失效时，daemon 先关闭发送门控，再丢弃旧 session 的 pending batch 和发送队列。daemon 可使用最近一次 CLI 注入的 endpoint 按配置执行轻量重连；重连期间继续丢弃采集结果。新握手完成后建立新的计数与 sequence 边界，再重新开放发送门控，不重放旧 session 或断连期间的数据。

VSOCK 会话建立后由 gateway 分配非零 `u32 sb-id`。`sb-id` 只标识当前 gateway 内的一条存活 SB 连接，不写入 SB observation。每个有效 ObservationBatch 都是连接活性信号；只有连续没有 observation 的时间达到 `max_silence_interval` 时，SB 才发送一次空 payload Heartbeat。gateway 收到任意有效入站帧都刷新连接活性时间，Heartbeat 不产生回复。旧 worker观察到 EOF、错误或 idle timeout并关闭、释放会话槽后，旧 `sb-id` 不再表示活跃session；重连建立新会话并获得新的 `sb-id`。

### 2.2 `actrail-vsock-gateway`

`actrail-vsock-gateway` 是 Host 上的 VSOCK-to-TCP proxy。它维护一条到 `actraild` 的上游 TCP 会话，并承载多个下游 `actrail-sb` VSOCK 会话：

```text
多个 SB VSOCK 会话
    ↓ 分配连接级 sb-id
有界转发
    ↓ 构造含 sb-id 和原始 SB frame 的 upstream frame
一条 gateway TCP 上游会话
```

Guest 始终通过标准 AF_VSOCK 连接 Host CID 与目标 port。

Host listener 由 backend adapter 提供。

Firecracker 是部署主线。

gateway 根据 microVM 的 `uds_path` 与目标 port 形成 `${uds_path}_${port}` Unix endpoint。

native AF_VSOCK 与 Cloud Hypervisor Unix endpoint 作为并列可选 backend。

backend 选择不改变 gateway session、转发或 upstream runtime。

gateway 与 `actraild` 建立 TCP 会话时发送 hello，由 daemon 分配非零 `u32 gateway-id`。`gateway-id` 只在 welcome 中返回并由 daemon 绑定到当前 TCP 连接，不重复写入 heartbeat 或数据消息。TCP 连接断开后该 ID 立即失效；上游重连获得新的 `gateway-id`。

gateway 接受 SB VSOCK 连接时完成 SB hello/welcome，会话内分配非零 `sb-id`。转发 observation 时，gateway 构造独立 upstream `ForwardedSbFrame`；其 payload 为固定 4-byte `sb-id` 加未经修改的完整 SB frame。gateway 处理两跳协议的 frame 边界与控制消息，但不解码 observation、不修改内层 SB frame、不执行插件匹配、不做进程、trace、identity 或 semantic 关联，也不持久化数据。

gateway 在上游空闲时发送空 payload heartbeat，daemon 只刷新连接活性而不回复。TCP 上游断开时 gateway 清除旧 `gateway-id` 并执行有界重连；现存 SB collector 不随上游断开而退出。单个 SB 转发失败或转发队列容量耗尽只终止对应 SB 连接，不终止 VSOCK listener 或其他 SB 会话，也不向 Guest 采集热路径传播阻塞。

每个 SB 会话拥有独立的有界转发额度，gateway 同时拥有全局有界上游队列。单个 SB 用尽自身额度时只拒绝该 SB 的后续输入，不能占用其他 SB 的保留额度。

### 2.3 `actraild / GatewayIngestRuntime`

`actraild` 在独立线程中监听 gateway TCP 地址。每个已接受的 gateway TCP 连接由独立连接线程顺序处理：

```text
HandObservationTcpListener thread
    └── GatewayConnection thread
        ├── gateway hello / welcome
        ├── heartbeat
        ├── frame 边界恢复与校验
        └── GatewayIngestRuntime 路由
```

daemon 为每条有效 gateway 连接分配非零 `u32 gateway-id`。`gateway-id` 在 daemon 的存活 gateway 连接集合内唯一；断连时 daemon 清除连接注册并使该连接下的全部 `(gateway-id, sb-id)` 来源立即失效，失效后的数字允许复用。连接线程从上游消息恢复 `sb-id + 原始 SB frame`，要求内层恰好包含一个有效的 observation batch，并以当前连接的 `gateway-id` 和消息携带的 `sb-id` 组成来源标记。来源标记只用于该独立 Hand 通路内的会话区分，不转换为脑侧 identity 或 trace membership。

`GatewayIngestRuntime` 对每条 observation 查询当前插件消费意向：

- 至少一个已加载插件声明消费该 observation 类型时，将 observation 投递给所有匹配插件。
- 没有已加载插件声明消费该 observation 类型时，将 observation 写入独立的 `Sandbox Evidence DB`。

插件投递和独立数据库写入是互斥路由；有匹配插件的 observation 不同时写入数据库。`Sandbox Evidence DB` 使用独立文件、schema、连接和生命周期，不属于 AcTrail 主 Storage。

只有一次成功的消费意向查询明确返回“无匹配插件”时才进入独立数据库。意向查询失败、匹配插件投递失败均不得伪装成无消费意向并改投数据库；独立数据库写入失败也不得改投插件或主 Storage。

Hand observation 不进入 `actraild` 的现有关联逻辑。gateway 连接异常、SB frame 异常、插件消费失败或独立数据库写入失败均以当前连接、当前插件或当前持久化操作为故障域，不终止 daemon，也不影响脑侧观测与治理链路。

## 3. Observation 模型

### 3.1 进程 I/O 计数

`ProcessIoCounters` 表达一个采样区间内某个根谱系的聚合增量：

```text
ProcessIoCounters
├── guest_boot_id
├── process
│   ├── pid
│   ├── start_time_ticks
│   └── executable_name
├── sample_started_ms
├── sample_ended_ms
├── read_operations
├── read_bytes
├── write_operations
├── write_bytes
├── failed_read_operations
└── failed_write_operations
```

### 3.2 Guest 资源快照

`GuestResourceSnapshot` 表达某个采样时刻的 Guest 环境状态：

```text
GuestResourceSnapshot
├── guest_boot_id
├── sampled_at_ms
├── cpu
│   ├── total_ticks
│   ├── idle_ticks
│   └── logical_cpu_count
└── memory
    ├── total_bytes
    ├── available_bytes
    ├── used_bytes
    └── oom_kill_count
```

CPU 使用方通过相邻累计快照计算区间利用率。OOM 消费方通过 `oom_kill_count` 的增量识别实际 OOM kill。

## 4. 插件消费

Sandbox observation 插件以 observation 类型声明消费意向。插件包可由 `actraild` 的启动配置显式加载，不依赖 `actrailctl`。声明快照在加载或卸载插件时更新；数据路由读取不可变快照，不在热路径调用插件探测接口。

匹配插件通过各自的有界队列异步消费 observation。一个插件队列已满、处理失败或退出时，只影响该插件，不阻塞 gateway 连接线程，也不改变其他插件的投递。

基础资源告警插件消费进程 I/O 计数和 Guest 资源快照，并按 `(gateway-id, sb-id)` 维护有界状态，产生以下告警：

- `OomKilled`：相邻快照中的 `oom_kill_count` 增加。
- `OomRisk`：可用内存字节数低于配置阈值。
- `HighCpu`：相邻 CPU 累计快照形成的区间利用率越过配置阈值。
- `HighRead`：采样区间读取字节数超过配置阈值。
- `HighWrite`：采样区间写入字节数超过配置阈值。

插件输出带类型的 `SandboxAlert`，通过有界、非阻塞的告警输出边界提交，不生成 JSON、trace 或 semantic action。告警在独立 Sandbox Alert DB 提交成功后，才尝试交给 builtin forwarding plugin。告警数据库、外发队列或 proxy 的运行期故障不得反向形成 Hand observation 消费错误。

阈值和来源状态容量由插件配置拥有。告警数据库和 writer 参数由独立 `SandboxAlertsConfig` 拥有。外发选择由 builtin forwarding plugin 配置拥有。

## 5. 传输协议

VSOCK 与 TCP 均使用紧凑二进制协议，不使用 JSON。所有 frame 使用固定长度 header、固定消息 code、显式 payload 长度和有界 payload。接收端在分配 payload 前校验 magic、协议版本、消息类型和长度上限，并正确处理半帧及连续多帧。

### 5.1 SB VSOCK 协议

```text
SbHello
SbWelcome(sb-id)
Heartbeat
ObservationBatch(sequence, observations)
```

`sb-id` 只出现在 gateway 返回的 `SbWelcome` 中。后续 SB heartbeat 和 observation frame 不携带 `sb-id`。

`SbHello`、`Heartbeat` 必须使用空 payload。hello 只接收一次，gateway 只对有效 hello 返回一次 welcome；heartbeat 不产生回复。非法消息、非空 heartbeat 或重复 hello 关闭对应 VSOCK 连接。

### 5.2 Gateway TCP 协议

```text
GatewayHello
GatewayWelcome(gateway-id)
Heartbeat
ForwardedSbFrame(sb-id, original_sb_frame)
```

`gateway-id` 只出现在 daemon 返回的 `GatewayWelcome` 中。后续 gateway heartbeat 和数据 frame 不携带 `gateway-id`。`ForwardedSbFrame` 的 payload 由 4-byte `sb-id` 和未经修改的完整 SB frame 构成。

`GatewayHello`、`Heartbeat` 必须使用空 payload。hello 只接收一次，daemon 只对有效 hello 返回一次 welcome；heartbeat 不产生回复。非法消息、非空 heartbeat 或重复 hello 关闭对应 TCP 连接。

协议只包含建立会话、保活和单向 observation 上报，不包含 Brain→Hand 动作、采集配置下发、`actrail-sb` CLI 控制或 `actrailctl` 控制消息。Guest-local control socket 是独立管理面，不复用 SB VSOCK wire protocol。

## 6. 生命周期与故障边界

### 6.1 启动

`actrail-sb daemon` 依次完成静态配置校验、单实例锁、Guest 能力校验、独立 eBPF load/attach、资源采样器初始化、采集线程与发送设施初始化和 Guest-local control socket bind。必要能力、对象或本地控制地址无效时启动失败，不使用其他 collector 冒充成功。daemon 以 `connected=false` 进入 ready；VSOCK endpoint 不属于 daemon 启动前置条件。

`actrail-sb connect` 校验运行时 host CID 与 port，通过 control socket 请求 daemon 建立 VSOCK session。请求校验失败时，本次 CLI 命令失败且不改变已有 endpoint、session 或发送门控。合法的新 endpoint 请求进入替换流程后，connect/hello/welcome 任一步失败只使本次命令失败；daemon 保持运行，采集设施继续工作，发送门控保持关闭。

gateway 在接收 SB 前完成配置校验、VSOCK bind 和 daemon TCP 初始连接。listener 地址冲突、上游配置无效或容量配置无效时启动失败。

`actraild` 在对外就绪前完成 Hand TCP bind、协议限制、独立数据库和已配置 Sandbox observation 插件初始化。配置无效或必要资源不可用时启动失败，不回退到既有 ingest listener 或主 Storage。

### 6.2 运行

- `actrail-sb` 的单次资源读取失败不停止 eBPF 采集；目标进程退出不停止资源采样；传输断开不卸载采集器。
- VSOCK 连接状态只控制 observation 是否进入发送队列。未连接和重连期间不缓存、不持久化、不补发。
- gateway 以 SB VSOCK 连接和 daemon TCP 连接为故障域，任一连接故障不终止进程中的 listener。
- `actraild` 以 gateway 连接线程、插件消费者和独立数据库操作为故障域；Hand 通路故障不得导致主进程退出，也不得传播到脑侧通路。
- `actrail-sb daemon` 随 Guest 生命周期常驻。正常运行阶段不积攒等待凑批的数据。收到终止信号时立即关闭发送门控，丢弃尚未完整发送的 observation，随后停止采集线程并释放连接、control socket、eBPF 资源和实例锁。退出不等待数据排空，也不为退出流程建立重连。
- daemon主线程通过`signalfd + ppoll`等待`SIGINT`/`SIGTERM`、control server health fd和可选diagnostics deadline。
- control server运行期退出时，主线程只输出一次集中诊断并移除其health fd；daemon采集和当前data session继续运行。
- 停止时先停止control admission，再关闭agent runtime并使发送门控失效，最后回收control poll owner。
- 周期诊断默认关闭。显式配置非零周期后，主线程在该deadline到达时读取累计计数并统一写入标准错误；关闭诊断时没有周期唤醒。采集与发送路径不直接格式化或输出诊断，诊断不进入 VSOCK observation 通路。
- gateway 与 `actraild` 关闭时停止接收新连接，终止生产者，排空各自允许排空的有界队列，再释放连接和存储资源。

## 7. 配置所有权

```text
SbDaemonConfig
├── instance_lock_path
├── collector
│   ├── root_process_names
│   ├── procfs_root
│   ├── require_initial_root
│   ├── root_refresh_interval_ms
│   ├── tracked_process_capacity
│   ├── pending_io_capacity
│   ├── aggregate_capacity
│   └── poll_interval_ms
├── sampler
│   └── poll_interval_ms
├── observation_queue
│   └── capacity
├── sender
│   ├── batch_max_observations
│   ├── io_timeout_ms
│   ├── max_silence_interval_ms
│   ├── reconnect_interval_ms
│   └── worker_thread_stack_bytes
├── control
│   ├── socket_path
│   ├── socket_mode_octal
│   ├── request_timeout_ms
│   ├── accepted_connection_max
│   ├── max_frame_bytes
│   └── worker_thread_stack_bytes
└── diagnostics
    └── interval_ms

SbConnectInvocation
├── control_socket
├── host_cid
├── port
├── request_timeout_ms
└── max_frame_bytes

VsockGatewayConfig
├── vsock_backlog
├── listener_backend
│   ├── firecracker_uds_path_and_port
│   ├── native_cid_and_port
│   └── cloud_hypervisor_socket_path
├── daemon_tcp_address
├── max_sb_connections
├── per_sb_forward_quota
├── outbound_queue_capacity
├── sb_peer_idle_timeout
├── upstream_heartbeat_interval
├── io_timeout
├── reconnect_interval
├── accept_poll_interval
└── connection_thread_stack_bytes

HandObservationConfig
├── enabled
├── tcp_listen_address
├── max_gateway_connections
├── accept_poll_interval
├── connection_poll_interval
├── gateway_idle_timeout
├── write_timeout
├── read_buffer_bytes
└── connection_thread_stack_bytes

SandboxEvidenceConfig
├── database_path
├── schema_version
├── write_batch_size
├── write_flush_interval
├── write_queue_capacity
├── busy_timeout
├── retention_policy
└── storage_capacity_limit

SandboxResourceAlertConfig
├── cpu_usage_threshold_basis_points
├── memory_available_threshold_bytes
├── read_interval_threshold_bytes
├── write_interval_threshold_bytes
└── source_state_capacity

SandboxAlertsConfig
├── enabled
├── database_path
├── schema_version
├── write_batch_size
├── write_flush_interval
├── write_queue_capacity
├── busy_timeout
├── retention_policy
├── storage_capacity_limit
└── writer_thread_stack_bytes
```

`SbDaemonConfig` 是随 daemon 和快照固定的静态配置。`SbConnectInvocation` 中的 endpoint 是快照恢复后由 CLI 注入的运行时参数，不写回静态配置，也不进入 observation。

各份配置由对应进程或组件独立拥有。采样配置变化不影响 gateway 与 daemon 配置；运行时 SB endpoint 变化不影响 collector 或 observation schema；gateway 连接配置变化不影响 observation schema；Hand listener 容量变化不改变 Guest 采集行为；独立数据库配置不影响 AcTrail 主 Storage；告警阈值变化不影响持久化、采集和传输配置。
