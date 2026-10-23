状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/daemon/src/services/ # 将采集、身份、语义投影、存储与安全治理接入宿主运行循环。
├── [x] wiring.rs # 构造存储、trace runtime 与接入服务依赖。
├── [x] attach.rs # 定义接入服务持有的采集器、存储、身份和治理组件。
│   └── [x] struct StorageAttachService # 统一拥有采集与持久化服务，协调已有进程接入、launch 接入和事件处理。
├── [x] attach/ # 接入服务的构建、权限预检、插件管理和 launch 准备。
│   ├── [x] factory.rs # 按配置构造采集器、存储和下游服务。
│   ├── [x] service.rs # 实现 AttachService 的进程接入与生命周期操作。
│   ├── [x] preflight.rs # 启动前核验 eBPF 所需能力。
│   │   └── [x] struct EbpfPreflightReport # 汇总采集器预检结果供接入判断。
│   ├── [x] plugin_config.rs # 校验插件配置与 profile 范围。
│   │   └── [x] struct PluginConfigManager # 组织插件配置检查与应用。
│   ├── [x] plugin_configuration.rs # 将插件配置请求转换为运行组件配置。
│   ├── [x] plugins.rs # 处理插件加载、卸载和命令分发。
│   ├── [x] debug.rs # 生成接入服务与采集器诊断快照。
│   ├── [x] logging.rs # 记录接入和初始化诊断。
│   ├── [x] helpers.rs # 处理接入流程共用的身份、路径和配置转换。
│   └── [x] launch/ # 在目标继续运行前准备 launch 身份与采集条件。
│       └── [x] preparation.rs # 准备 launch 所需进程信息和运行依赖。
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
│   ├── [x] service.rs # 组织 TLS 同步队列、运行依赖和完成诊断。
│   │   └── [x] struct TlsSyncService # 协调计划解析、目标对象固定和同步请求排空。
│   ├── [x] resolver.rs # 为 exec 解析所需 TLS 采集计划。
│   │   └── [x] struct TlsSyncPlanResolver # 结合目标进程信息与运行配置生成同步计划。
│   ├── [x] root_path.rs # 固定目标进程根目录及可访问对象路径。
│   ├── [x] direct.rs # 发现目标对象并执行直接同步任务。
│   ├── [x] launch.rs # 固定 launch 使用的同步计划和对象。
│   └── [x] frame_buffer.rs # 缓冲同步协议帧并保留未完整读取的数据。
├── [x] live.rs # 组织运行期事件排空和各服务推进顺序。
├── [x] live/ # 处理直接采集、seccomp、身份收敛与关闭阶段。
│   ├── [x] batch.rs # 按批推进采集事件与持久化。
│   ├── [x] reconcile.rs # 对齐采集到的进程事实与 trace 注册状态。
│   ├── [x] direct.rs # 排空直接采集来源的事件。
│   ├── [x] seccomp.rs # 排空 seccomp 相关事件与治理完成结果。
│   ├── [x] launch_binding.rs # 将 launch 观测与预先建立的 trace 身份关联。
│   ├── [x] shutdown.rs # 协调关闭阶段的停止接纳与数据排空。
│   └── [x] shutdown/ # 约束关闭阶段推进与剩余等待时间。
│       └── [x] progress.rs # 记录关闭阶段、截止时间和阶段探测信息。
├── [x] semantic_actions.rs # 向语义投影提供当前 trace 记录查询。
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
