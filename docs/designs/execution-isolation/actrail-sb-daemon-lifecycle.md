# actrail-sb Daemon 与连接控制设计

## 1. 同一 binary 的三个入口

```text
actrail-sb daemon --config <path>
actrail-sb connect --control-socket <path> --host-cid <cid> --port <port>
actrail-sb init --output <path>
```

`daemon` 是 Guest 内的单实例常驻进程。

它拥有 Guest-only eBPF、资源采样、Connection Gate、有界发送设施、VSOCK Session Owner 和 Guest-local Control Server。

`connect` 是短生命周期 Control Client。

它只向已经运行的 daemon 提交运行时 endpoint，并等待本次命令结果。

`init` 从 binary 内置的默认 profile 生成静态 daemon TOML。

CLI 不加载 eBPF，不创建采集线程，不持有 VSOCK data session，也不经过 `actrailctl`。

Guest-local control socket 与 SB VSOCK data protocol 是两个独立边界。

## 2. 快照前 daemon 启动

Firecracker 模板 Guest 在制作快照前启动 daemon。

daemon 按以下顺序初始化：

1. 加载并严格校验静态配置。
2. 阻塞 `SIGINT` 与 `SIGTERM`，创建 `signalfd`。
3. 构造 VSOCK transport factory 和 Guest-local Control Server 配置。
4. 获取 Guest 单实例锁。
5. 加载Guest-only eBPF programs，创建并校验maps；标准tracepoint通过perf event ioctl兼容路径attach，任一必要tracepoint失败则启动失败。
6. 初始化 procfs resource reader。
7. 建立显式 I/O baseline，并完成首次 resource sample。
8. 预分配 observation queue 与 session pending batch。
9. 创建 I/O、resource 和 VSOCK Session Owner workers。
10. bind Guest-local control socket，启动非阻塞 control poll owner 与异步 dispatcher。
11. 以 `connected=false`、`publication_enabled=false` 进入 ready。

启动时 baseline 任一 collector failure 都使 daemon 启动失败。

daemon ready 不要求 gateway 存在，也不要求提供 VSOCK host CID 或 port。

快照包含已加载的 eBPF、maps、采集器、workers、预分配内存、control listener 和实例锁。

快照不包含有效 VSOCK connection、`sb_id` 或部署实例 endpoint。

## 3. 未连接时的采集行为

```text
Disconnected  → publication_enabled = false
Connecting    → publication_enabled = false
Connected     → publication_enabled = true
Reconnecting  → publication_enabled = false
```

Connection Gate 只控制 observation admission，不控制采集设施生命周期。

`publication_enabled = false` 时：

- eBPF programs、maps 和 links 保持有效。
- I/O worker 正常读取并推进累计计数 baseline。
- resource worker 按独立周期读取 Guest CPU、memory 和 OOM 状态。
- observation 在 queue 之前立即丢弃。
- 不创建 session batch，不写 Guest 文件或数据库，不等待未来连接补发。
- Session Owner 没有 active session 时不发送 ObservationBatch 或 Heartbeat。

producer 在采样前捕获 connection generation。

采样结束后只有 generation 仍为当前值时才允许 `try_send`，避免旧 session 的采样结果跨越连接边界。

## 4. Guest-local control request

CLI 构造一个有界 Connect request：

```text
Connect
├── control_socket
├── host_cid
├── port
├── request_timeout
└── max_frame_bytes
```

Control Client 连接 Guest-local UDS，写入一个二进制 command frame，关闭写方向并等待一个 response frame。

Control Server 的 poll owner 同时维护 listener、stop fd、dispatcher wake fd 和有界 accepted connection set。

每条 connection 只接受一个 command，并持有固定 request/response buffer 上限和 connection deadline。

frame limit 必须能容纳最大 rejection frame。

poll owner 不执行 VSOCK connect、handshake 或 baseline。

完整 command 通过单槽有界 channel 非阻塞交给单 worker dispatcher。

dispatcher 已在执行 command 时，新的合法 command 立即返回 `Busy`。

因此慢 VSOCK connect 不阻塞 listener 接收、frame 读取、超时回收或 Busy response。

CLI request timeout 限制调用方等待。

CLI timeout 只限制 CLI 等待响应，不撤销 daemon 已接受的 Connect。

daemon 的 control timeout 独立限制服务端一次操作的占用时间。

Session Owner 串行处理 Connect；处理中再次调用返回 `Busy`，完成后对相同端点的重试幂等返回当前 session。

runtime 为命令建立 completion state；请求过期或 CLI 已取消后，Session Owner 不得提交新 active session。

## 5. 建立 VSOCK session

Session Owner 串行处理 Connect：

1. 校验 runtime host CID 与 port。
2. 保持 Connection Gate 关闭。
3. 使旧 active session 和 reconnect target 失效。
4. 丢弃旧 pending batch 与 observation queue。
5. 创建 AF_VSOCK connection并执行 `SbHello/SbWelcome`。
6. 校验 gateway 返回的非零 `sb_id`。
7. 请求 I/O worker读取并丢弃当前 aggregate，建立新的显式 baseline。
8. 丢弃 baseline 期间进入旧 queue 的 observation。
9. 分配新的非零 connection generation，重置 batch sequence 为 1。
10. 在 request 仍允许提交时发布 active session并开放 Connection Gate。
11. 向 CLI 返回 `sb_id`、generation 与 reused 标志。

baseline cycle 存在任何 collector failure 时，本次连接失败。

gate 保持关闭，daemon 和采集设施继续运行。

同一 endpoint 已有效连接时，重复 Connect 返回现有 session，不重建 VSOCK connection。

不同 endpoint 的 Connect 替换现有 session。

## 6. 已连接发送与 Heartbeat

Connection Gate 使用原子 generation。

I/O 与 resource workers 只对当前 generation 执行有界 `try_send`。

queue 满、关闭或 generation 改变时立即丢弃当前 observation，不阻塞 producer。

Session Owner 收到第一条 observation 后立即发送。

它只合并当时已经就绪的 observation，最多达到 `batch_max_observations`，不等待凑满。

只有 active session 连续没有写出任何 observation frame达到 `max_silence_interval` 时，才发送空 Heartbeat。

正常资源快照持续到达时，Heartbeat 不触发。

## 7. 断连与重连

ObservationBatch 或 Heartbeat 写失败时，Session Owner：

1. 关闭 Connection Gate。
2. 清除当前 `sb_id` 与 active connection。
3. 丢弃 pending batch 和旧 queue。
4. 保存最近一次 runtime endpoint 作为 reconnect target。
5. 按 `reconnect_interval` 执行轻量 reconnect。

重连期间 producer继续采集，但 observation在gate前丢弃。

每次重连成功都重新完成 handshake、显式 I/O baseline、旧 queue 丢弃、generation 推进和 sequence 重置，然后开放 gate。

旧 session、断连期和重连期数据不补发。

新的 CLI Connect 可替换 reconnect target。

## 8. 主线程事件与诊断

daemon main 使用一次阻塞 `ppoll` 同时等待：

- `signalfd` 上的 `SIGINT` 或 `SIGTERM`。
- Control Server health fd 的退出事件。
- diagnostics 启用时的下一次输出 deadline。

`diagnostics.interval_ms = 0` 时没有 diagnostics timeout，也没有固定周期 wake loop。

启用 diagnostics 时，main 在 deadline 到达后统一读取 runtime/collector 原子累计值并写标准错误。

采集、gate、session 与 control command 路径不直接格式化周期诊断。

Control Server poll owner运行期退出时，health fd产生终止事件。

main回收该server结果并输出一次集中诊断，随后不再轮询该health fd。

daemon采集和当前VSOCK data session保持运行；新的CLI Connect不可用。

## 9. 静态配置与运行时参数

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
│   ├── oom_event_capacity
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

SbRuntimeEndpoint
├── host_cid
└── port
```

CLI 的 request timeout 与 frame limit 默认值复用 daemon 默认 profile。

CLI request 不写回 daemon TOML。

## 10. 停止行为

收到 shutdown signal 后：

1. app先请求Control Server停止listener admission和poll owner。
2. app关闭Sandbox Agent Runtime。
3. runtime关闭Connection Gate，标记stopping并唤醒workers。
4. Session Owner丢弃pending与queue，关闭VSOCK connection，不再重连。
5. I/O/resource/session workers退出并join；collector drop释放BPF links/maps。
6. app最后join Control Server poll owner。
7. process owner离开作用域，释放control socket与实例锁。

停止过程不排空 observation，不发起重连，也不等待 gateway 或 `actraild` 确认。
