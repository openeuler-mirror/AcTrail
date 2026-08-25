# 执行隔离采集观测端到端时序

## 进程启动顺序

Firecracker 模板制备与沙箱运行时分离：

```text
0. 模板 Guest 内启动 actrail-sb daemon，完成采集设施预热后制作快照
1. 启动 actraild
2. 配置运行时 VSOCK endpoint 并启动 actrail-vsock-gateway
3. 从快照恢复 Firecracker microVM
4. Guest 内执行 actrail-sb connect，控制 daemon 连接运行时 VSOCK endpoint
5. 启动被观测的目标进程及其子进程
```

`actrail-sb daemon` 在快照前完成独立 eBPF collector、资源采样器、采集线程、发送设施和 Guest-local control listener 初始化，但不建立 VSOCK 连接。

`actraild` 必须先完成插件、独立 Sandbox Evidence DB 和 Hand TCP listener 初始化。

gateway 必须先完成到 `actraild` 的 TCP 注册，之后才开放 Guest link listener。

快照恢复后的 `actrail-sb connect` 只向已经运行的 daemon 注入 host CID 与 port，并等待 daemon 完成 `SbHello/SbWelcome`。

StratoVirt/Kata 不要求复用 Firecracker 快照机制：Guest 启动后先运行同一 daemon，等待
本地 ready，再通过 `actrail-sb connect` 提交 Host CID 与 port。StratoVirt 经 gateway 的
native AF_VSOCK backend 接入；除 endpoint 解析与 Guest 启动方式外，control、Session
Owner、Connection Gate、gateway session 和上游路由时序保持不变。

## 完整时序

```mermaid
sequenceDiagram
    autonumber
    actor Operator as 部署/沙箱管理器
    participant Daemon as actraild 启动与生命周期
    participant PluginHost as Sandbox Plugin Facade
    participant AlertPlugin as sandbox-resource-alert
    participant AlertDb as Sandbox Alert DB
    participant Forwarding as builtin alert-forwarding
    participant AlertProxy as actraild-alert-proxy
    participant Evidence as Sandbox Evidence DB
    participant HandListener as Hand TCP Listener
    participant DaemonWorker as Gateway Connection Worker
    participant Gateway as actrail-vsock-gateway
    participant GatewaySb as Gateway SB Session
    participant Firecracker as Firecracker VMM
    participant SbCli as actrail-sb CLI
    participant ControlClient as Guest Control Client
    participant ControlServer as Guest Control Server
    participant Dispatcher as Control Dispatcher
    participant Sb as actrail-sb Process Owner
    participant SbRuntime as Sandbox Agent Daemon Owner
    participant SbSession as VSOCK Session Owner
    participant Resource as Guest Resource Reader
    participant Ebpf as Guest-only eBPF Collector
    participant Workload as 目标根进程及其后代

    rect rgb(255, 248, 232)
        Note over Operator,Ebpf: 阶段零：Firecracker 模板 Guest 预热
        Operator->>Sb: actrail-sb daemon --config ...
        Sb->>Sb: 加载静态配置
        Sb->>Sb: block SIGINT/SIGTERM并创建signalfd
        Sb->>Sb: 校验静态配置并获取Guest单实例flock
        Sb->>Ebpf: load/校验/attach独立sandbox BPF object并创建maps
        Ebpf-->>Sb: maps、links与初始I/O基线ready
        Sb->>Resource: 初始化procfs reader并完成首次读取校验
        Resource-->>Sb: resource reader ready
        Sb->>SbRuntime: 创建Connection Gate、预分配queue/batch并启动I/O、resource、session workers
        SbRuntime-->>Sb: ready(connected=false)
        Sb->>ControlServer: bind Guest-local UDS并启动非阻塞poll owner
        ControlServer->>Dispatcher: 启动单槽有界、非阻塞admission的单worker dispatcher
        ControlServer-->>Sb: listener ready + health fd
        Sb-->>Operator: daemon ready(connected=false)
        Operator->>Firecracker: 制作包含预热daemon的microVM快照
        Note over Sb,Ebpf: daemon ready不依赖VSOCK endpoint；快照中没有有效VSOCK stream或sb_id
    end

    rect rgb(235, 242, 255)
        Note over Operator,HandListener: 阶段一：Host daemon 启动
        Operator->>Daemon: actraild --config operator.conf run
        Daemon->>Daemon: 校验配置、PID、Host ID 与 daemon wiring
        Daemon->>AlertDb: 打开独立 SQLite、校验 schema、推进 ingest_epoch
        AlertDb-->>Daemon: writer ready + read probe success
        Daemon->>PluginHost: 加载 startup plugins 和 persistent plugins
        PluginHost->>AlertPlugin: 校验 manifest、selector、容量与插件配置
        AlertPlugin-->>PluginHost: 注册 process-io / guest-resource 消费意向
        Daemon->>Evidence: 打开独立 SQLite、校验 schema、推进 ingest_epoch
        Evidence-->>Daemon: writer ready + read probe success
        Daemon->>HandListener: bind hand_observation.listen_addr
        HandListener->>HandListener: nonblocking listen + 启动 accept thread
        Daemon->>Daemon: bind 原有 control UDS 并完成 serve loop 状态初始化
        Daemon-->>Operator: actraild ready
        Daemon->>Daemon: 进入 control UDS serve loop
        Note over Daemon,HandListener: startup plugin 的 FailFast 失败使启动失败，Continue 只记录并继续；persistent plugin 任一加载失败使启动失败；Evidence DB、Hand TCP bind/accept-thread 或 control UDS bind 失败也使启动失败；listener 不会早于 DB ready
    end

    rect rgb(238, 250, 240)
        Note over Operator,GatewaySb: 阶段二：Host gateway 启动
        Operator->>Firecracker: 配置guest_cid与VSOCK base uds_path
        Operator->>Gateway: actrail-vsock-gateway --config ...
        Gateway->>Gateway: 校验 backend、地址、容量、超时和线程栈
        Gateway->>HandListener: TCP connect
        HandListener->>DaemonWorker: accept 后创建独立 connection worker
        Gateway->>DaemonWorker: GatewayHello(empty)
        DaemonWorker->>DaemonWorker: 预占连接槽并分配非零 gateway_id
        DaemonWorker-->>Gateway: GatewayWelcome(gateway_id)
        Gateway->>Gateway: 创建全局有界 upstream queue 并启动 TCP sender
        Gateway->>GatewaySb: bind backend-neutral Guest link listener
        Note over Gateway,GatewaySb: Firecracker主线监听${uds_path}_${port}
        Note over Gateway,GatewaySb: native AF_VSOCK与Cloud Hypervisor UDS使用各自可选backend
        GatewaySb->>GatewaySb: 启动 VSOCK accept thread 与 SessionRegistry
        Gateway-->>Operator: gateway ready(snapshot.gateway_id)
        Note over Gateway,GatewaySb: 初始 TCP connect、GatewayHello/Welcome、VSOCK bind/nonblocking 或 accept-thread 创建失败时 gateway 启动失败，不在未注册状态宣告 ready
        Note over Gateway: 初始 Welcome 必须携带非零 ID；若 upstream 在 ready snapshot 前断开，打印的 snapshot.gateway_id 可以暂时为 0
    end

    rect rgb(255, 248, 232)
        Note over Operator,Sb: 阶段三：恢复Guest并由CLI控制daemon建立运行时连接
        Operator->>Firecracker: 从预热快照恢复microVM
        Firecracker-->>Operator: Guest恢复；actrail-sb daemon保持connected=false
        Operator->>SbCli: actrail-sb connect --control-socket guest-control-socket --host-cid runtime-host-cid --port runtime-vsock-port
        SbCli->>ControlClient: 构造有界Connect command与CLI等待timeout
        ControlClient->>ControlServer: Guest-local binary Connect frame
        ControlServer->>ControlServer: 非阻塞读取、frame上限校验与connection deadline
        alt dispatcher空闲
            ControlServer->>Dispatcher: try_dispatch(command)
            Dispatcher->>SbRuntime: SandboxControlPort::execute(command)
            SbRuntime->>SbSession: try_send带daemon control timeout与completion state的Connect
            SbSession->>SbSession: 校验endpoint；保持publication_enabled=false并丢弃旧session pending与queue
            SbSession->>Firecracker: AF_VSOCK connect(CID=runtime-host-cid, port=runtime-vsock-port)
            Firecracker->>GatewaySb: AF_UNIX connect(${uds_path}_${port})
            GatewaySb->>GatewaySb: accept 后创建独立 SB connection worker
            SbSession->>Firecracker: SbHello(empty)
            Firecracker->>GatewaySb: 原样桥接SbHello
            GatewaySb->>GatewaySb: 预占 session 与 per-SB quota，分配非零 sb_id
            GatewaySb-->>Firecracker: SbWelcome(sb_id)
            Firecracker-->>SbSession: 原样桥接SbWelcome
            SbSession->>SbRuntime: 请求I/O worker建立失败感知baseline
            SbRuntime-->>SbSession: baseline success
            SbSession->>SbSession: 丢弃旧queue；建立generation与sequence边界
            SbSession->>SbSession: request仍在deadline内时发布stream与sb_id；publication_enabled=true
            SbSession-->>SbRuntime: ConnectResponse(success, sb_id, generation)
            SbRuntime-->>Dispatcher: control response
            Dispatcher-->>ControlServer: wake poll owner并交付response
            ControlServer-->>ControlClient: bounded response frame
            ControlClient-->>SbCli: success
            SbCli-->>Operator: connect成功
        else 已有control command正在执行
            ControlServer-->>ControlClient: Rejected(Busy)
            ControlClient-->>SbCli: Busy
        end
        Note over ControlClient,SbSession: CLI timeout只结束CLI等待；daemon串行防重入，同端点重试幂等
        Note over SbCli,SbRuntime: connect失败只使CLI失败；daemon和预热采集设施继续运行，publication_enabled保持false
        Note over SbCli,Firecracker: Firecracker profile使用运行时Host CID（默认2）；VSOCK port由本次runtime endpoint确定
    end

    rect rgb(247, 242, 255)
        Note over Workload,Sb: 阶段四：Guest 内独立采集
        loop 目标进程生命周期与 I/O
            Workload->>Ebpf: fork/vfork/clone
            Ebpf->>Ebpf: 后代继承根 lineage marker
            Workload->>Ebpf: read/write syscall enter + exit
            Ebpf->>Ebpf: tracked PID 命中后累计成功/失败次数与成功字节数
            Workload->>Ebpf: kernel OOM mark_victim
            Ebpf->>Ebpf: 捕获victim PID/comm并在事件时查询lineage map
            Ebpf->>Ebpf: try_push有界OOM event queue
            Workload->>Ebpf: process exit
            Ebpf->>Ebpf: 清理退出 PID；根聚合由用户态轮询后回收
        end

        par I/O 采集线程
            loop io_poll_interval
                Sb->>Ebpf: 刷新命名根与现有后代
                alt root refresh 成功
                    Ebpf-->>Sb: root set refreshed
                else root refresh 失败
                    Ebpf-->>Sb: 记录 failure，仍继续读取已有 lineage aggregate
                end
                Sb->>Ebpf: 读取 lineage 聚合增量
                Sb->>Ebpf: 有界排空OOM victim queue
                alt 存在 I/O 增量
                    Ebpf-->>Sb: ProcessIoCounters[]
                    alt publication_enabled=true
                        Sb->>Sb: try_send每条ProcessIo observation
                    else 未连接或重连中
                        Sb->>Sb: 推进I/O基线并立即丢弃observation
                    end
                else 无增量
                    Ebpf-->>Sb: 空集合，不产生 observation
                else aggregate collect 或采样时钟失败
                    Ebpf-->>Sb: 记录 failure，本轮不产生 I/O observation
                end
                opt 存在OOM victim事件
                    Ebpf-->>Sb: OomVictimObservation[]
                    alt publication_enabled=true
                        Sb->>Sb: try_send每条OomVictim observation
                    else 未连接或重连中
                        Sb->>Sb: 立即丢弃victim observation
                    end
                end
            end
        and Guest 资源采样线程
            loop resource_poll_interval
                Sb->>Resource: sample CPU / memory / oom_kill累计计数
                alt 采样成功
                    Resource-->>Sb: GuestResourceSnapshot
                    alt publication_enabled=true
                        Sb->>Sb: try_send GuestResource observation
                    else 未连接或重连中
                        Sb->>Sb: 立即丢弃resource observation
                    end
                else 本轮采样失败
                    Resource-->>Sb: failure，本轮跳过并继续下一轮
                end
            end
        end

        alt 已连接且observation queue有容量
            Note over Sb: 采集线程不阻塞；observation 进入 sender 队列
        else 已连接但observation queue已满
            Note over Sb: 丢弃当前 observation；不阻塞 eBPF 或资源采样
        else 未连接或重连中
            Note over Sb: observation不进入队列、不落盘、不等待补发
        end
    end

    rect rgb(232, 250, 250)
        Note over Sb,DaemonWorker: 阶段五：即时发送、VSOCK proxy 与 TCP 转发
        loop actrail-sb sender仅在VSOCK session有效时发送
            alt 队列出现首条 observation
                Sb->>Sb: 立即唤醒；合并当时已就绪项，最多 batch_max
                Sb->>Sb: 编码 ObservationBatch(sequence++)
                Sb->>Firecracker: Frame::ObservationBatch
                Firecracker->>GatewaySb: 原样桥接ObservationBatch
                GatewaySb->>GatewaySb: 任意有效帧刷新 SB last_activity
                GatewaySb->>Gateway: 保留完整内层 SB frame，封装 ForwardedSbFrame(sb_id, frame)
                Gateway->>Gateway: 获取 per-SB quota 并 try_send 全局有界 upstream queue
                Gateway->>DaemonWorker: TCP Frame::ForwardedSbFrame
            else 队列为空且未达到 max_silence_interval
                Note over Sb,GatewaySb: 等待 observation；不发送 Heartbeat
            else 连续无 observation 达到 max_silence_interval
                Sb->>Firecracker: Heartbeat(empty)
                Firecracker->>GatewaySb: 原样桥接Heartbeat
                GatewaySb->>GatewaySb: 刷新 last_activity，不转发给 daemon
            end
        end

        loop upstream queue 暂时为空且距上次 upstream Heartbeat/重连达到 interval
            Gateway->>DaemonWorker: 独立 upstream Heartbeat(empty)
            DaemonWorker->>DaemonWorker: 刷新 TCP connection last_activity，不进入路由
        end
        Note over Sb,GatewaySb: 正常资源快照持续产生 ObservationBatch，因此 SB 最大静默 Heartbeat 通常不会触发
    end

    rect rgb(255, 242, 242)
        Note over DaemonWorker,Evidence: 阶段六：actraild 独立 Hand 路由
        DaemonWorker->>DaemonWorker: 校验外层 frame、sb_id 与完整内层 ObservationBatch
        DaemonWorker->>PluginHost: deliver(gateway_id, sb_id, batch)
        PluginHost->>PluginHost: 按每条 observation kind 查询不可变消费意向快照

        alt 整批均无插件消费意向
            PluginHost->>Evidence: try_append(source, sequence, full batch)
            Evidence->>Evidence: 独立 writer 异步写入 Sandbox Evidence SQLite
        else 至少一条 observation 有匹配插件
            loop 每个匹配插件 consumer
                PluginHost->>AlertPlugin: try_publish ConsumerBatch(该插件匹配的 observation indices)
                alt 插件队列 Accepted
                    loop ConsumerBatch 中每条匹配 observation
                        AlertPlugin->>AlertPlugin: 按 (gateway_id, sb_id) 更新有界状态
                        opt CPU 区间利用率进入超阈值状态
                            AlertPlugin->>AlertDb: try_append HighCpu
                        end
                        opt OOM victim事件到达
                            AlertPlugin->>AlertDb: try_append OomKilled
                        end
                        opt available_bytes 从非风险状态跌破阈值
                            AlertPlugin->>AlertDb: try_append OomRisk
                        end
                        opt read_bytes 超过区间阈值
                            AlertPlugin->>AlertDb: try_append HighRead
                        end
                        opt write_bytes 超过区间阈值
                            AlertPlugin->>AlertDb: try_append HighWrite
                        end
                        Note over AlertPlugin: 同一 observation 可以产生多个满足条件的告警；无条件命中时只更新必要状态
                    end
                else 插件队列 Full/Closed
                    AlertPlugin-->>PluginHost: 当前 consumer admission 失败
                end
            end
            opt 同批中存在无匹配的 observation
                PluginHost->>Evidence: try_append 仅未匹配 observation
            end
            opt 至少一个告警成功入队
                AlertDb->>AlertDb: 独立 SQLite 批量事务提交结构化告警
                AlertDb->>Forwarding: post-commit 标准化外发副本
                alt 外发有效且类别匹配
                    Forwarding->>AlertProxy: ATAP v2 ForwardAlert(source.sandbox)
                    AlertProxy->>AlertProxy: 对匹配订阅者执行非阻塞 fanout
                else 外发 disabled、断开或 queue full
                    Forwarding->>Forwarding: 丢弃外发副本，保留数据库记录
                end
            end
        end
        Note over PluginHost,Evidence: 匹配的 observation 只投匹配插件；同批未匹配 observation 写 Evidence；整批 NoInterest 时全批写 Evidence。任何失败都不触发跨分支 fallback
    end

    rect rgb(245, 245, 245)
        Note over Sb,Evidence: 阶段七：运行期 fail-local 与重连
        alt SB VSOCK 写失败或连接断开
            SbSession->>SbSession: publication_enabled=false
            SbSession->>SbSession: 丢弃当前pending与旧session queue
            loop 直到重连成功或进程停止
                SbSession->>Firecracker: AF_VSOCK reconnect(CID=runtime-host-cid, port=runtime-vsock-port)
                Firecracker->>GatewaySb: AF_UNIX reconnect(${uds_path}_${port})
                SbSession->>Firecracker: SbHello(empty)
                Firecracker->>GatewaySb: 原样桥接SbHello
                GatewaySb-->>Firecracker: 新 SbWelcome(new sb_id)
                Firecracker-->>SbSession: 原样桥接SbWelcome
            end
            SbSession->>SbRuntime: 建立新I/O baseline
            SbSession->>SbSession: 建立新generation和sequence边界；publication_enabled=true
            Note over SbSession,GatewaySb: 重连期间采集结果直接丢弃；不重发旧session、断连期间或重连期间的数据。新VSOCK session获得新sb_id；其他SB session不受影响
        else Guest-local Control Server异常退出
            ControlServer-->>Sb: health fd终止事件
            Sb->>ControlServer: try_result并移除health fd
            Sb->>Sb: 集中输出一次control unavailable诊断
            Note over Sb,SbSession: collector、sampler与当前data session继续运行；新的CLI Connect不可用
        else per-SB quota 或 gateway 全局 queue 已满
            GatewaySb->>GatewaySb: 结束当前 SB worker并关闭该 VSOCK session
            Note over GatewaySb,Gateway: 当前未入队 batch，以及该 SB 已入 upstream queue 但尚未发送的 frame 均可能丢失；其他 SB session 保持运行
        else gateway TCP 写失败
            Gateway->>Gateway: 保留当前 ForwardItem
            loop upstream 重连
                Gateway->>HandListener: TCP reconnect
                Gateway->>DaemonWorker: GatewayHello
                DaemonWorker-->>Gateway: GatewayWelcome(new gateway_id)
            end
            Gateway->>DaemonWorker: 重发当前 ForwardedSbFrame
            Note over DaemonWorker: 新 TCP connection 获得新 gateway_id；旧 worker 观察到连接关闭或 idle 后释放连接槽，旧 gateway_id 不再表示活跃 connection
        else daemon 协议或 frame 解码错误
            DaemonWorker->>DaemonWorker: 只关闭当前 gateway TCP connection
        else daemon sink 交付失败
            DaemonWorker->>DaemonWorker: 记录并丢弃当前 batch，继续读取同一 TCP connection
        else 插件队列 Full/Closed
            PluginHost->>PluginHost: 同步 admission 仅标记当前 consumer 失败
            PluginHost->>Evidence: 仍独立执行同批未匹配 observation 的 Evidence admission
            Note over PluginHost,Evidence: 已匹配 observation 不因 admission 失败而回退 Evidence
        else 插件已入队后的 consume 或 Sandbox Alert DB 失败
            AlertPlugin->>AlertPlugin: 异步记录当前插件 batch/alert 失败并继续后续工作
            Note over AlertPlugin,Evidence: Evidence 路由早已完成；不补写、不 fallback，其他插件独立
        else Evidence admission 为 Full/Closed/TooLarge
            Evidence-->>PluginHost: 同步拒绝当前 archive batch
            Note over PluginHost,Evidence: 不改投插件或主 Storage
        else Evidence 异步 SQLite transaction 失败
            Evidence->>Evidence: 记录 failed_batches 与 last_error；该 transaction 中已接纳 batch 不重试
            Evidence->>Evidence: writer 继续处理后续已接纳工作
        end
    end

    opt 沙箱生命周期结束
        Operator->>Sb: SIGTERM
        Sb->>Sb: signalfd readable；ppoll返回shutdown event
        Sb->>ControlServer: request_stop，先停止listener admission
        Sb->>SbRuntime: shutdown
        SbRuntime->>SbRuntime: publication_enabled=false并丢弃pending与发送队列
        SbRuntime->>Ebpf: I/O worker退出并随collector drop释放BPF links/maps
        SbRuntime->>Resource: resource worker退出
        SbRuntime->>SbSession: 停止session，不重连
        Sb->>ControlServer: join非阻塞poll owner
        Sb->>Sb: 释放control socket与Guest单实例锁
        Operator->>Gateway: SIGTERM
        Gateway->>GatewaySb: 停止 VSOCK accept 与活动 sessions
        Gateway->>DaemonWorker: 关闭 TCP upstream
        Operator->>Daemon: SIGTERM
        Daemon->>HandListener: 停止 accept 与 connection workers
        Daemon->>Evidence: 排空允许排空的有界写入并关闭独立 DB
        Daemon->>PluginHost: 关闭 daemon services 与插件 consumers
        Daemon->>AlertDb: 排空允许排空的告警并关闭独立 DB
        Daemon->>Daemon: 删除 control UDS runtime file 与 PID file
    end
```

## 时序不变量

- `gateway_id` 由 `actraild` 在每次 TCP `GatewayHello` 后分配；TCP 重连获得新 ID。
- `sb_id` 由 gateway 在每次 VSOCK `SbHello` 后分配；VSOCK 重连获得新 ID。
- daemon 内的来源键是 `(gateway_id, sb_id)`；它不转换成脑侧 identity、trace 或 semantic 关联。
- `ObservationBatch.sequence` 由每个 SB sender 从 1 递增；Evidence store 使用独立持久化 `ingest_epoch` 区分数据库重启前后的证据身份。
- Guest-side eBPF、procfs reader、maps、links 和采集线程均由 `actrail-sb` 独立拥有，与 `actraild` 的 eBPF 采集无关。
- daemon 在快照前完成采集设施初始化；CLI 只通过 Guest-local control socket 注入运行时 VSOCK endpoint。
- Guest-local Control Server 的poll owner不执行VSOCK连接；单worker dispatcher执行control command，已有命令执行时并发命令返回Busy。
- daemon main通过signalfd、Control Server health fd和可选diagnostics deadline事件驱动等待；diagnostics关闭时没有周期wake loop。
- VSOCK session 有效性只门控 observation 发布；未连接、断连和重连期间的数据不入队、不落盘、不补发。
- Firecracker、Cloud Hypervisor 和经 native AF_VSOCK 接入的 StratoVirt 共享同一 Session Owner、gateway session、forwarder 与 upstream runtime；VMM 适配不改变数据语义。
- `actrail-vsock-gateway` 只处理连接、frame、ID、quota 和转发，不解码 observation payload，不执行插件匹配或持久化。
- Hand observation 不经过现有 Ingest、Identity、Trace、Semantic、Recording、Export 或主 Storage，也不需要 `actrailctl`。
