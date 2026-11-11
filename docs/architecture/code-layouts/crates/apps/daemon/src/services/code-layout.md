状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/daemon/src/services/ # 将采集、身份、语义投影、存储与安全治理接入宿主运行循环。
├── [x] wiring.rs # 构造依赖后一次准备配置要求的观测与治理能力并集，完成后报告启动就绪。
├── [x] attach.rs # 定义接入服务持有的采集器、存储、身份和治理组件。
│   └── [x] struct StorageAttachService # 统一拥有采集与持久化服务，协调已有进程接入、launch 接入和事件处理。
├── [x] attach/ # 接入服务的构建、权限预检、插件管理和 launch 准备。
│   ├── [x] factory.rs # 按整体配置构造采集器及加载模块，保留空需求下的无内核资源状态。
│   ├── [x] service.rs # launch 和 attach 根据准备结果选择后端，同配置复用长期 runtime。
│   ├── [x] preflight.rs # 从实际配置汇总需求并准备一次 runtime，再只读检查各候选 Required 与实际 LSM 结果。
│   │   └── [x] struct EbpfPreflightReport # 提供 profile 可用性快照，准备状态由 collector 的 RuntimePreparation 持有。
│   ├── [x] plugin_config.rs # 校验插件配置与 profile 范围。
│   │   └── [x] struct PluginConfigManager # 组织插件配置检查与应用。
│   ├── [x] plugin_configuration.rs # 将插件配置请求转换为运行组件配置。
│   ├── [x] plugins.rs # 处理插件加载、卸载和命令分发。
│   ├── [x] debug.rs # 生成接入服务与采集器诊断快照。
│   ├── [x] logging.rs # 记录接入和初始化诊断。
│   ├── [x] helpers.rs # 处理接入流程共用的身份、路径和配置转换。
│   ├── [x] watched.rs # 为其他 daemon 服务关注 Trace：复制其文件事件，保存等待放行的 LLM 请求，排空后一并取走。
│   │   ├── [x] struct HeldLlmRequest # 被关注 Trace 的一个 LLM 请求：hold、Trace 和 llm.request 动作 ID，不含请求内容。
│   │   └── [x] struct WatchedTraces # 被关注和已结束关注的 Trace、文件事件副本和被扣住的 LLM 请求。
│   └── [x] launch/ # 在目标继续运行前准备 launch 身份与采集条件。
│       └── [x] preparation.rs # 复用已准备能力并登记 launch 目标，必需能力缺失时拒绝当前请求。
├── [x] identity/ # 连接 procfs 身份、trace 注册表与控制请求身份。
│   └── [x] service.rs # 解析进程身份并应用运行时进程事件。
│       ├── [x] struct TraceIdentityResolver # 从进程事实解析 trace 关联身份。
│       ├── [x] struct RuntimeProcessEventApplier # 将进程生命周期事件应用到运行时身份状态。
│       └── [x] struct ControlActorIdentityResolver # 解析控制操作主体对应的进程身份。
├── [x] application_protocol/ # 将网络负载交给 HTTP 协议分析并生成应用事件。
│   ├── [x] service.rs # 管理协议分析状态与应用事件输出。
│   │   ├── [x] struct ApplicationProtocolAnalyzer # 持有协议分析器并处理接入的负载段。
│   │   └── [x] struct ApplicationEventDraft # 表达待关联身份、分配 ID 和持久化的应用事件。
│   ├── [x] http1.rs # 适配 HTTP/1 流解析结果。
│   └── [x] http2.rs # 适配 HTTP/2 流解析结果。
├── [x] payload.rs # 组织负载策略、重排和持久化阶段。
├── [x] payload/ # 维护负载接纳规则、敏感内容处理与事件写入顺序。
│   ├── [x] policy.rs # 从采集配置解析负载处理策略。
│   ├── [x] reorder.rs # 在有界窗口内重新排列负载片段。
│   │   └── [x] struct PayloadSegmentReorderer # 持有待排序片段并按流推进可处理序列。
│   ├── [x] redaction.rs # 对负载执行隐私字段处理。
│   ├── [x] transaction.rs # 组织负载与关联事件的持久化步骤。
│   └── [x] transaction/ # 准备语义事件与负载段的关联写入。
│       └── [x] semantic_persistence.rs # 将应用事件和负载段整理为可持久化记录。
├── [x] payload_gate/ # 判断 socket 负载与响应正文是否需要保留。
│   ├── [x] body_retention.rs # 按应用协议语义裁定正文保留。
│   │   └── [x] struct PayloadBodyRetentionGate # 保存正文保留判定所需的流状态。
│   ├── [x] socket.rs # 筛选需要进入 HTTP 分析的 socket 负载。
│   │   └── [x] struct SocketHttpPayloadGate # 管理 socket HTTP 识别与负载接纳状态。
│   └── [x] socket/ # 识别 HTTP CONNECT 建立的隧道。
│       └── [x] connect.rs # 推进客户端 CONNECT 协商与隧道判定。
├── [x] control_runtime.rs # 将控制策略插件接到宿主执行环境。
│   └── [x] struct ControlPluginRuntime # 持有控制插件运行状态并分发治理调用。
├── [x] command_control/ # 命令执行请求的策略判定、审计与工作线程。
│   └── [x] service.rs # 装配命令治理依赖并处理命令请求。
│       ├── [x] struct CommandControlService # 管理命令策略执行与审计生命周期。
│       └── [x] struct CommandControlBackend # 持有命令策略执行后端所需的共享运行依赖。
├── [x] network_control/ # 网络连接请求的身份解析、策略判定和审计。
│   └── [x] service.rs # 装配网络策略并处理拦截请求。
│       ├── [x] struct NetworkControlService # 管理网络治理请求与策略执行。
│       └── [x] struct NetworkControlBackend # 持有网络策略执行后端所需的共享运行依赖。
├── [x] enforcement/ # 对文件访问执行权限判定并生成拦截审计。
│   └── [x] service.rs # 管理文件治理运行依赖与请求排空。
│       └── [x] struct FileEnforcementService # 持有文件治理状态并驱动文件权限事件处理。
├── [x] access_control/ # 将 LSM 访问结果投影为审计记录与边界告警。
│   ├── [x] grey.rs # LsmPluginDecider 在调用插件前查询权威进程索引并传入已知逻辑进程 ID，未关联时保留缺失状态。
│   ├── [x] descriptor.rs # 描述 LSM 访问控制运行能力。
│   │   └── [x] struct LsmAccessControl # 提供 LSM 访问控制描述及配置适配入口。
│   ├── [x] audit.rs # 将内核访问事实映射为统一审计事件。
│   └── [x] alerts.rs # 将权限越界事实转换为告警。
├── [x] process_seccomp/ # 识别 seccomp 进程创建与 exec 通知。
│   └── [x] service.rs # 解析进程系统调用并准备进程观测信息。
│       └── [x] struct ProcessSeccompService # 协调进程通知、身份准备与执行候选信息。
├── [x] seccomp_notify/ # 管理 seccomp 通知读取、延后与继续执行。
│   └── [x] service.rs # 驱动通知处理和 continuation 生命周期。
│       └── [x] struct SeccompNotifyService # 持有通知监听器及待处理通知。
├── [x] seccomp_socket/ # 从 seccomp 通知获取 socket 访问及负载事实。
│   └── [x] service.rs # 协调 socket 请求读取、身份关联与捕获。
│       └── [x] struct SeccompSocketService # 管理 socket 通知的采集状态。
├── [x] seccomp_tls/ # 将 seccomp 执行时机接入 TLS 同步准备。
│   └── [x] service.rs # 处理 TLS 相关通知并协调目标进程恢复。
│       └── [x] struct SeccompTlsService # 持有 TLS 通知处理所需运行状态。
├── [x] tls_sync/ # 在目标运行前解析 TLS 库路径并同步采集准备。
│   ├── [x] service.rs # 协调每进程连接、Trace 配额、epoll 就绪读取和计划查询。
│   │   └── [x] struct TlsSyncService # 协调计划解析、目标对象固定和同步请求排空。
│   ├── [x] clients.rs # 接入时保留 socket 凭据，以登记帧中的 TLS 事件凭据授权事件，发送进程退出后仍接收缓冲事件；绑定 Trace 并执行配额和单轮读取预算，断开时交出已上报 stream 的 close；计划查询仍解析活进程身份。
│   ├── [x] limits.rs # 计算连接与 fd 预算，并在启动时提高 RLIMIT_NOFILE。
│   ├── [x] release.rs # 放行 checkpoint launch 的 runtime：按 TLS 连接放行到已处理的出站序号，LLM 请求的 hold 只在同一 Trace 中匹配连接，并停在它的末尾序号之前；放行消息入队后回收没有 hold 的进度条目；Trace 结束后让仍在上报的进程不再等待；未写出的放行超过 1 MiB 时关闭该连接。
│   │   └── [x] struct ClientReleases # 一条上报连接上各 TLS 连接已处理、已放行的序号和 hold，以及待写出的放行帧。
│   ├── [x] resolver.rs # 为 exec 解析所需 TLS 采集计划。
│   │   └── [x] struct TlsSyncPlanResolver # 结合目标进程信息与运行配置生成同步计划。
│   ├── [x] resolver_worker.rs # 从有界队列解析计划并回复，直接持有查询 socket；跳过已断开的请求。
│   ├── [x] root_path.rs # 固定目标进程根目录及可访问对象路径。
│   ├── [x] direct.rs # 发现目标对象并执行直接同步任务。
│   ├── [x] launch.rs # 固定 launch 使用的同步计划和对象。
│   └── [x] frame_buffer.rs # 缓冲未完整读取的帧，大帧消费完后释放超过 1 MiB 的容量。
├── [x] checkpoint/ # 将 checkpoint 协调接入 launch、控制命令和 LLM 边界。
│   ├── [x] service.rs # 装配协调器及其记录库写连接、只读连接和完成队列。
│   │   └── [x] struct CheckpointService # 持有协调器和等待初始保存的 launch；每次排空时应用完成结果、把 LLM 请求交给协调器，交出可以放行的 hold，把已离开 TraceRuntime 或进入终态的 Trace 记为已结束，协调器随之 disconnect；仍在运行的 Trace disconnect 完成后不再关注它的 LLM 请求。
│   ├── [x] completions.rs # 工作线程的完成结果经队列交给 daemon，并写 eventfd 唤醒主循环。
│   ├── [x] launch.rs # 检查非 root 发起者只保护自己拥有的目录；从仍停在启动屏障的根进程读取启动时间、namespace 和工作目录，组成 launch 请求。
│   ├── [x] commands.rs # 把 checkpoint 控制命令转给协调器的命令和查询，经协调器找出命令所属的 Trace 及其所有者，把记录转为视图，直接回答当前 daemon 的暂停与工作线程状态。
│   ├── [x] procfs.rs # 读取 /proc 中的父进程、状态、线程数和启动时间，区分僵尸主线程与已退出的整个线程组。
│   └── [x] process_control.rs # 从根和已知成员发现并暂停存活后代；仅有 namespace 坐标的成员在暂停时匹配 PID 与启动代，退出代不阻止恢复；根退出仍覆盖被重新收养的成员，只继续由 checkpoint 暂停的进程。
├── [x] live.rs # 组织运行期事件排空和各服务推进顺序；TLS-sync 的已读明文处理完后再处理上报连接断开产生的 stream close。
├── [x] live/ # 处理直接采集、seccomp、身份收敛与关闭阶段。
│   ├── [x] batch.rs # 按批推进采集事件与持久化。
│   ├── [x] reconcile.rs # 对齐采集到的进程事实与 trace 注册状态。
│   ├── [x] held_llm_requests.rs # payload 识别出被关注 Trace 的 LLM 请求时，立即按 stream_key 与末尾序号扣住，排空后交给 checkpoint；其余已处理的流量由 payload 事务在写入存储之前放行。
│   ├── [x] direct.rs # 排空直接采集来源的事件。
│   ├── [x] seccomp.rs # 排空 seccomp 相关事件与治理完成结果。
│   ├── [x] launch_binding.rs # 将 launch 观测与预先建立的 trace 身份关联。
│   ├── [x] shutdown.rs # 协调关闭阶段的停止接纳与数据排空。
│   └── [x] shutdown/ # 约束关闭阶段推进与剩余等待时间。
│       └── [x] progress.rs # 记录关闭阶段、截止时间和阶段探测信息。
├── [x] semantic_actions.rs # 持久化语义批次、处理交付结果并查询 trace 记录。
│   └── [x] struct LiveTraceRecordLookup # 借用运行时与存储事实，解析语义记录依赖。
├── [x] resource_profiling/ # 适配资源组件的工作负载身份、控制记录与生命周期；详见目录布局。
├── [x] retention.rs # 扫描并清理超过保留策略的存储数据。
│   └── [x] struct StorageRetentionService # 持有保留策略和扫描调度状态。
├── [x] post_trace/ # 在 trace 完成后接纳并执行后处理插件工作。
│   ├── [x] coordinator.rs # 校验后处理接纳条件并汇总执行结果。
│   │   └── [x] struct PostTraceCoordinator # 协调后处理任务、完成结果与问题记录。
│   └── [x] broker.rs # 管理后处理请求和宿主能力调用。
│       └── [x] struct PostTraceBroker # 持有后处理请求分发状态。
├── [x] alert_ingress/ # 接纳插件及系统边界告警并校验来源权限。
│   ├── [x] broker.rs # 管理告警接纳与排队。
│   │   └── [x] struct AlertIngress # 持有告警入口与待持久化队列。
│   └── [x] system.rs # 将文件、命令越界事实聚合为系统告警。
├── [x] alert_forwarding/ # 将已接纳告警转交告警代理连接。
│   └── [x] service.rs # 驱动转发连接与告警发送。
│       └── [x] struct AlertForwardingService # 持有告警转发进度及连接状态。
├── [x] sandbox_plugins/ # 管理沙箱插件实例、能力路由和配置。
│   ├── [x] manager.rs # 启动、查找和关闭沙箱插件实例。
│   │   └── [x] struct SandboxPluginManager # 拥有沙箱插件实例及实例生命周期。
│   └── [x] route_sink.rs # 将沙箱插件输出接入宿主消费端。
│       └── [x] struct SandboxPluginRouteSink # 承接沙箱插件路由输出。
├── [x] sandbox_alerts/ # 将沙箱产生的告警送入宿主告警入口。
│   └── [x] forwarder.rs # 转换并转发沙箱告警。
│       └── [x] struct SandboxAlertForwarder # 管理沙箱告警接收与宿主转发。
├── [x] logging/ # 汇总采集诊断和工作负载健康信息。
│   ├── [x] diagnostic.rs # 记录采集和处理过程中产生的诊断。
│   └── [x] workload.rs # 维护按工作负载组织的运行诊断。
│       └── [x] struct WorkloadDiagnostics # 保存工作负载观测状态与诊断计数。
└── [x] user_lifecycle.rs # 将用户会话和回合通知应用到 trace 运行状态。
```
