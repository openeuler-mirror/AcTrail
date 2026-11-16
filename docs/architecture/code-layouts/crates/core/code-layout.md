状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/ # 领域状态与运行协调；应用组合具体采集器、存储和传输端口。
├── [x] config/ # 组合 operator 配置，生成默认文档并校验采集能力及模块配置。
├── [x] config_export_filter/ # 选择交付配置与 Web 编辑字段；调用方提供所属模块模板并执行领域配置校验。
│   ├── [x] Cargo.toml # 声明 serde 与保留键顺序的 TOML 依赖。
│   ├── [x] allowlist.toml # 编译内嵌的选择策略，定义默认模式及独立校验的白名单、黑名单。
│   ├── [x] build.rs # 内嵌策略文本或读取错误，跟踪策略文件及目录变更。
│   │   └── [x] struct PolicyBundle # 将读取结果写入 OUT_DIR/policy.rs，策略缺失时仍允许构建。
│   └── [x] src/ # 策略校验、选定字段查询与配置投影。
│       ├── [x] lib.rs # 导出 ConfigExportFilter。
│       └── [x] filter.rs # 数组整体选择，按输入文档顺序投影叶子字段。
│           └── [x] struct ConfigExportFilter # 保存当前模式及唯一的不可变选定路径集合。
│               ├── [x] bundled # 读取内嵌策略并以所属模块模板校验两份名单，明确返回缺失、非法或未知路径错误。
│               ├── [x] project # 复制选中叶子；白名单裁剪空父表，黑名单保留空表。
│               ├── [x] paths # 按路径顺序迭代选定字段，供调用方构建编辑字段目录。
│               └── [x] allows # 查询字段是否属于当前选定集合。
├── [x] model/ # 定义跨采集、语义、控制和存储边界传递的领域值。
│   └── [x] src/ # 身份、trace、事件、能力、载荷与诊断模型。
├── [x] process_identity/ # 将不同 PID 坐标收敛到稳定身份；身份解析不拥有 trace membership。
│   └── [x] src/ # 身份值、来源端口及索引维护。
│       ├── [x] lib.rs # 导出身份模型、读取端口和管理器。
│       ├── [x] contract.rs # 区分逻辑身份、observer 坐标和 namespace alias。
│       │   ├── [x] struct ProcessIdentity # AcTrail 分配的稳定数值 ID，供事件、membership 和存储关联。
│       │   ├── [x] struct HostProcessCoordinates # daemon 可见 PID、开始时间与可选线程信息。
│       │   ├── [x] struct NamespaceProcessCoordinates # namespace 身份、其中的 PID 与进程代际。
│       │   ├── [x] struct ProcessObservation # 一次身份读取中的 observer 坐标和可选 namespace 坐标。
│       │   ├── [x] struct ProcessRecord # 同一稳定身份的坐标、alias 集合与解析状态。
│       │   └── [x] trait ProcessIdentityReader # 根据可见 PID 返回身份观测或明确的读取错误。
│       └── [x] manager.rs # 维护身份记录及精确坐标索引，处理 alias 补充和活跃 PID 复用。
│           ├── [x] struct ProcessIdentityManager # 拥有身份分配区间、记录、代际索引和活跃 PID 索引。
│           │   ├── [x] resolve_or_create # 查找唯一候选，分配或补充身份，并返回被替换的活跃映射。
│           │   ├── [x] install_reserved_block # 在当前区间耗尽后接入连续预留的 ID 区间。
│           │   └── [x] mark_exited # 移除活跃坐标映射并保留可供历史关联的身份记录。
│           ├── [x] struct ProcessResolution # 返回身份、是否创建或补充及被置换的活跃身份。
│           └── [x] enum ProcessIdentityError # 区分 ID 耗尽、多个候选和 observer 坐标冲突。
├── [x] trace_runtime/ # 管理运行中的 trace、进程归属与生命周期；持久化由应用边界协调。
│   └── [x] src/ # 生命周期注册表、membership 索引与采集能力协商。
│       ├── [x] lib.rs # 导出 TraceRuntime 及各运行时协作模块。
│       ├── [x] registry.rs # 拥有活跃 trace，并依据根进程及后代状态推进生命周期。
│       │   ├── [x] struct TraceRuntime # 持有 trace ID 分配器、collector 描述和 TraceEntry 注册表。
│       │   │   ├── [x] create_starting_trace # 创建 trace、根 membership、告警令牌和导出 trace 身份。
│       │   │   ├── [x] activate_trace # 将已准入 trace 及其初始 membership 转为活跃状态。
│       │   │   ├── [x] insert_observed_child # 登记已发生的 fork 关系，不因后续传播开关变化拒绝历史事实。
│       │   │   ├── [x] track_remove_root # 关闭根进程采集及传播，依据剩余成员推进收尾。
│       │   │   └── [x] complete_after_resource_barrier # 在资源控制完成后结束 draining trace，开放成员使健康状态降级。
│       │   ├── [x] struct TraceEntry # 聚合 TraceRecord、固定 profile、sensor plan、membership 与 owner。
│       │   └── [x] struct TraceOwnerPrincipal # 记录准入主体 UID、PID namespace 和 mount namespace。
│       ├── [x] membership.rs # 按稳定 ProcessIdentity 查找和维护 trace 内进程归属。
│       │   └── [x] struct MembershipIndex # 保存根与后代 membership，提供活跃采集成员判定。
│       ├── [x] sensor_plan.rs # 将 profile 的必需与可选能力分配给支持它们的 collector。
│       │   ├── [x] struct SensorPlan # 保存 profile 名称、collector 能力分配及未绑定的可选能力。
│       │   └── [x] struct NegotiationFailure # 说明无法满足的能力与失败原因，交准入方处理。
│       ├── [x] state_machine.rs # 校验 trace 状态迁移，分别维护生命周期和健康状态。
│       │   └── [x] struct StateTransitionError # 返回非法迁移的原状态和目标状态。
│       └── [x] commands.rs # 接收创建 trace 与移除根进程的领域请求。
│           ├── [x] struct TrackTraceRequest # 携带根身份、profile 快照、标签及初始环境事实。
│           └── [x] struct RootRemovalRequest # 指定待停止传播的 trace 及发生时间。
├── [x] attach_runtime/ # 协调已有进程的采集绑定、树快照与身份合并，记录 bootstrap 覆盖情况。
│   └── [x] src/ # 接入协调器及快照合并步骤。
│       ├── [x] lib.rs # 导出接入请求、结果和 BootstrapCoordinator。
│       ├── [x] bootstrap.rs # 借用 collector、身份读取器和快照源，完成已有进程接入。
│       │   ├── [x] struct BootstrapCoordinator # 使用请求中的 sensor plan，编排身份读取、采集绑定与快照交接。
│       │   │   └── [x] attach_existing # 消费接入请求并返回 bootstrap 结果或带阶段的失败。
│       │   ├── [x] struct AttachRequest # 指定根进程、profile 和 trace 的初始属性。
│       │   └── [x] struct BootstrapResult # 返回 trace、根身份、接入时刻和 bootstrap 覆盖不完整标记。
│       ├── [x] coverage_guard.rs # 将采集与快照时刻转换为 bootstrap 覆盖窗口。
│       │   ├── [x] struct BootstrapTimings # 保存绑定、快照开始及结束等关键时间。
│       │   └── [x] struct CoverageWindow # 表达快照和实时采集衔接的覆盖范围。
│       ├── [x] identity_merge.rs # 对比快照与实时事件中的进程观察集合。
│       │   └── [x] struct IdentityMergeSummary # 区分快照独有、实时事件独有及两者共有的进程观察。
│       └── [x] snapshot_merge.rs # 将快照进程关系转换为 trace membership。
│           └── [x] struct SnapshotMergeResult # 汇总快照合并后的成员与覆盖结果。
├── [x] ingest_runtime/ # 将 collector 事件转为带身份、策略及 provider 分类的领域事件。
│   └── [x] src/ # 单事件摄入步骤；应用提供精确归属及记录 ID。
│       ├── [x] lib.rs # 组合规范化、观测策略与 provider 分类，返回事件和诊断。
│       │   ├── [x] struct IngestPipeline # 持有策略评估器并借用 provider classifier，组织一次处理。
│       │   │   └── [x] process # 缺少匹配时仅产身份诊断；策略允许后生成领域事件及可选标签事件。
│       │   ├── [x] struct IngestMatch # 由调用方确认的 trace、进程身份和可选父身份。
│       │   └── [x] struct IngestOutcome # 一次摄入生成的事件与诊断集合，交调用方继续投影和记录。
│       ├── [x] match_event.rs # 根据稳定身份解析事件对应的 membership。
│       ├── [x] normalize.rs # 将 raw collector 内容与已匹配身份组装为 DomainEvent。
│       ├── [x] policy_gate.rs # 对观测内容应用记录策略，形成允许、丢弃等结果。
│       ├── [x] classify.rs # 从已规范化事件提取 provider 证据并生成标签事件。
│       └── [x] diagnostics.rs # 为身份匹配失败及观测策略决定生成诊断记录。
├── [x] semantic_action_runtime/ # 将事件及明文协议投影为语义动作、关系、内容和 lineage。
├── [x] plugin_system/ # 定义插件能力、授权、宿主端口与 Sandbox 消费者注册表。
├── [x] plugin_wasm_runtime/ # 承载 WASM 实例、受限宿主调用与 observation/control/codec 适配。
├── [x] resource_profiling/ # 拥有 Host/Workload 资源采集、指标、存储、告警及配置。
├── [x] idle_detector/ # 读取 agent 的执行状态，按持续时间产生空闲告警与恢复事件。
│   └── [x] src/ # 检测配置、空闲片段状态及告警输出。
│       ├── [x] lib.rs # 导出 IdleDetector、配置和告警类型。
│       ├── [x] config.rs # 定义检测开关、轮询周期与空闲阈值。
│       │   └── [x] struct IdleDetectionConfig # 提供默认值并校验检测时间参数。
│       ├── [x] runtime.rs # 拥有轮询时钟及每个执行实例的空闲片段；执行事实来自 ExecutionStates。
│       │   ├── [x] struct IdleDetector # 依据权威执行状态计算空闲与恢复，不独立维护执行状态。
│       │   │   ├── [x] poll # 到期后扫描执行状态，输出本轮告警；未知状态不作为恢复依据。
│       │   │   └── [x] finish_trace # 清理已结束 trace 的空闲片段。
│       │   └── [x] struct Episode # 保存空闲起点的单调时刻和事件墙钟时间。
│       └── [x] alert.rs # 将检测结果转换为告警契约中的事件。
│           └── [x] struct IdleAlert # 关联 trace 和告警草稿；草稿携带执行实例、空闲时刻、阈值及恢复事件种类。
├── [x] checkpoint/ # 拥有 checkpoint 的 scope 规划、按 Trace 调度和 backend 组装；经端口调用记录库与进程控制。
├── [x] tls_payload_sync/ # 定义 TLS-sync 计划、注入环境、帧协议及事件发送客户端。
├── [x] sandbox_agent_runtime/ # 协调 Guest 采样线程、工作单元会话和有界观测投递。
├── [x] vsock_gateway_runtime/ # 将多个 Guest 的 VSOCK 连接转发到 daemon TCP 上游，限制会话与待发送帧。
│   └── [x] src/ # 网关生命周期、连接注册与上游发送队列。
│       ├── [x] lib.rs # 导出网关配置、运行时和状态快照。
│       ├── [x] config.rs # 校验监听、上游、帧长度、连接和线程资源限制。
│       │   └── [x] struct GatewayConfig # 持有 VSOCK 监听、TCP 上游及转发预算。
│       ├── [x] runtime.rs # 建立上游、监听 Guest 并管理接收线程及关闭流程。
│       │   ├── [x] struct GatewayRuntime # 拥有会话注册表、上游链路、停止信号和 accept 线程。
│       │   │   ├── [x] start # 校验配置并建立上游和监听，启动连接处理。
│       │   │   └── [x] shutdown # 通知线程退出，停止接收并回收上游运行资源。
│       │   ├── [x] struct SbConnectionWorker # 处理单个 Guest 的握手、帧校验和转发。
│       │   └── [x] struct GatewaySnapshot # 展示当前 gateway ID、活跃 Guest 数和转发计数。
│       ├── [x] session.rs # 将 Guest 会话接入与释放集中在同一注册表。
│       │   └── [x] struct SessionRegistry # 在容量限制内分配和释放 sb 会话身份。
│       └── [x] upstream.rs # 维护 TCP 上游与有界发送工作线程。
│           ├── [x] struct UpstreamLink # 拥有上游身份、发送端和工作线程的生命周期。
│           ├── [x] struct UpstreamSender # 将带会话来源的帧提交到发送队列。
│           ├── [x] struct SessionForwardQuota # 限制单 Guest 尚未发送的转发工作量。
│           └── [x] struct SessionForwardPermit # 随队列项生命周期归还会话额度。
├── [x] gateway_ingest_runtime/ # daemon 侧网关连接准入与批次交付；通过 sink 接入应用的兴趣路由。
│   └── [x] src/ # 连接生命周期、交付端口与运行计数。
│       ├── [x] lib.rs # 导出摄入运行时、连接句柄、sink 和状态快照。
│       ├── [x] runtime.rs # 用共享状态维护连接配额、网关 ID 分配及关闭请求。
│       │   ├── [x] struct GatewayIngestRuntime # 持有共享 RuntimeState，提供线程安全准入和状态读取。
│       │   │   ├── [x] try_open # 检查关闭状态及连接上限，分配非零 gateway ID。
│       │   │   └── [x] request_shutdown # 停止新连接准入，由传输层结束已接入连接。
│       │   ├── [x] struct GatewayConnection # 拥有一次连接的 gateway ID，释放时归还连接配额。
│       │   │   └── [x] deliver # 携带 gateway ID 和 sb ID 调用 sink，并累计成功或失败计数。
│       │   └── [x] enum GatewayOpenError # 区分配置非法、配额满、关闭中及 ID 耗尽。
│       ├── [x] sink.rs # 由应用实现的 Sandbox 观测交付边界。
│       │   ├── [x] trait SandboxObservationSink # 接收来源坐标与 ObservationBatch，返回局部交付结果。
│       │   └── [x] struct SinkDeliveryError # 保存交付失败阶段和说明。
│       └── [x] status.rs # 输出连接、心跳、批次数量及交付失败统计。
│           └── [x] struct GatewayIngestStatus # 运行计数的只读快照，供控制和诊断展示。
├── [x] linux_cgroup/ # 操作 Linux cgroup v1/v2 层级、目录及计数文件，向资源组件提供内核访问能力。
│   └── [x] src/ # 共同计数模型及按版本组织的文件系统适配。
│       ├── [x] lib.rs # 导出共同类型与 v1/v2 内核适配接口。
│       ├── [x] common.rs # 共同计数、范围路径、目录扫描、错误及文件读取辅助逻辑。
│       │   ├── [x] struct CgroupCounters # 汇总资源计数及各可选计数的可用情况。
│       │   ├── [x] struct TraceScopePaths # 描述一个 trace 的相对路径、aggregate 与 workload 路径。
│       │   ├── [x] struct ManagedScopeDirectory # 关联扫描出的 trace ID、nonce 与范围路径。
│       │   └── [x] enum CgroupError # 区分内核访问、目录结构与计数解析错误。
│       ├── [x] v1.rs # v1 层级发现、managed 范围生命周期与计数读取。
│       │   ├── [x] struct ManagedCgroupV1 # 在各 controller 层级创建范围、迁移进程、恢复并清理目录。
│       │   ├── [x] struct CgroupV1CounterReader # 读取 v1 CPU、内存及可选 pids/blkio 计数。
│       │   ├── [x] struct V1Hierarchy # 保存已发现的 controller 挂载层级。
│       │   └── [x] struct V1PreflightReport # 返回 v1 层级与 managed 能力预检结果。
│       └── [x] v2.rs # v2 委派预检、managed 范围生命周期与计数读取。
│           ├── [x] struct ManagedCgroupV2 # 持有受管根路径，创建和回收 trace 对应的内核目录。
│           │   ├── [x] prepare_hierarchy # 检查委派并准备 controller 层级。
│           │   ├── [x] create_trace_scope # 创建 aggregate 与 workload 目录。
│           │   ├── [x] move_pid # 将目标进程写入 workload 的成员列表。
│           │   ├── [x] scan_managed_scopes # 在数量预算内扫描可恢复的 trace 范围。
│           │   └── [x] remove_empty_trace_scope # 删除已无成员的 trace 范围。
│           ├── [x] struct CgroupV2CounterReader # 保留绑定目录句柄和 memory.peak 读取句柄，持续读取同一目录对象。
│           │   ├── [x] from_open_directory # 接收已核验目录句柄，避免通过显示路径重新绑定对象。
│           │   └── [x] read_counters # 读取 CPU、内存、I/O 及 populated 状态。
│           ├── [x] struct UnifiedHierarchy # 保存 cgroup2 挂载根和当前进程所属层级路径。
│           └── [x] struct CgroupPreflightReport # 返回委派所有权、controller、daemon 位置和探测结果。
└── [x] linux_platform/ # 提供 Linux 能力预检和 seccomp 系统调用映射，供控制与采集适配器使用。
    └── [x] src/ # 平台检查及架构相关系统调用编号转换。
        ├── [x] lib.rs # 导出平台模块，并将 linux_cgroup 重导出为 cgroup_v2。
        ├── [x] capability_probe.rs # 检查 Unix socket 与 no_new_privs 能力。
        │   └── [x] struct CapabilityStatus # 返回能力名称、是否可用和检查详情。
        ├── [x] file_seccomp.rs # 将配置中的文件系统调用映射为内核编号及路径参数位置。
        │   └── [x] enum KernelFileSeccompSyscall # 标识受支持的内核文件调用，解析 seccomp_data 中的调用类型。
        └── [x] process_seccomp.rs # 将进程控制配置展开为架构支持的内核系统调用。
            └── [x] enum KernelProcessSyscall # 转换进程调用编号和配置名称，统一调用匹配语义。
```
