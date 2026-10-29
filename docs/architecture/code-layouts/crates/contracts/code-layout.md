状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/contracts/ # 按采集、控制、语义、存储与隔离传输划分共享数据和能力接口
├── [x] collector/ # 活跃采集器的能力声明、trace 绑定、事件批次与统计
│   ├── [x] instance/ # 采集器实例生命周期接口
│   │   └── [x] src/ # 实例契约定义
│   │       └── [x] lib.rs # 采集器绑定、轮询和刷新传输接口
│   │           ├── [x] trait CollectorInstance # bind_trace、unbind_trace、poll_batch、flush_transport、stats 约束采集器行为
│   │           └── [x] struct CollectorPollBatch # 携带原始观测、明文段和明文流关闭记录
│   ├── [x] binding/ # 采集覆盖保护与 trace 绑定数据
│   │   └── [x] src/ # 绑定请求和返回句柄
│   │       └── [x] lib.rs # 定义请求的进程范围及生效绑定
│   │           ├── [x] struct CoverageGuardRequest # 请求目标进程的采集覆盖保护
│   │           ├── [x] struct TraceBindingRequest # 携带 trace 及其采集绑定条件
│   │           └── [x] struct TraceBindingHandle # 表达已建立的 trace 绑定
│   ├── [x] capability/ # 描述采集器支持范围与能力绑定失败
│   │   └── [x] src/ # 能力声明模型
│   │       └── [x] lib.rs # 采集器描述与失败详情
│   │           └── [x] struct CollectorDescriptor # 描述采集器名称和可提供的能力
│   ├── [x] event/ # 尚未完成用户态归属的原始事实
│   │   └── [x] src/ # 原始事件与来源封装
│   │       └── [x] lib.rs # 定义事件身份、信封和载荷
│   │           ├── [x] struct RawCollectorEvent # 携带采集得到的原始事件
│   │           ├── [x] struct RawEventEnvelope # 保留采集侧事件身份与公共字段
│   │           └── [x] enum RawObservationPayload # 区分各类原始观测载荷
│   └── [x] stats/ # 活跃绑定数与事件丢弃统计
│       └── [x] src/ # 采集统计模型
│           └── [x] lib.rs # 统计快照与丢弃计数
│               └── [x] struct CollectorStats # 表达采集器当前绑定和丢弃情况
├── [x] payload/ # TLS、socket 和 stdio 明文采集的能力及输入模型
│   ├── [x] binding/ # 明文采集绑定条件
│   │   └── [x] src/ # 明文绑定请求
│   │       └── [x] lib.rs # 定义目标与明文采集要求
│   │           └── [x] struct PayloadBindingRequest # 携带明文绑定所需的目标和选项
│   ├── [x] capability/ # 明文后端的能力类型与共享限额常量
│   │   └── [x] src/ # 明文能力定义
│   │       └── [x] capability.rs # 明文能力枚举
│   │           └── [x] enum PayloadCapability # 表达可请求的明文采集能力
│   └── [x] event/ # 明文段及流结束事实
│       └── [x] src/ # 原始明文输入模型
│           └── [x] segment.rs # 定义原始段与流关闭记录
│               ├── [x] struct RawPayloadSegment # 保留来源、流标识和采集字节
│               └── [x] struct RawPayloadStreamClose # 表达采集侧明文流结束
├── [x] control_plane/ # ctl 与 daemon 的进程注册、生命周期上报及插件管理命令
│   └── [x] src/ # 控制命令、响应和 trace 选择器
│       ├── [x] command.rs # 定义管理请求及 launch 相关输入
│       │   ├── [x] enum ControlCommand # 汇集进程跟踪、查询、插件和 Agent 生命周期命令
│       │   ├── [x] struct TrackAddCommand # 表达新增跟踪目标及 launch 上下文
│       │   └── [x] struct LaunchTlsProbePlan # 携带 launch 采用的 TLS 探针计划
│       ├── [x] reply.rs # 定义控制成功响应与错误
│       │   ├── [x] enum ControlReply # 区分跟踪、查询、诊断与插件操作结果
│       │   └── [x] struct LaunchTlsPlanReply # 返回 launch TLS 计划解析结果
│       └── [x] selector.rs # 选择控制操作作用的 trace
│           └── [x] enum TraceSelector # 表达支持的 trace 选择方式
├── [x] agent_lifecycle/ # Agent 轮次、用户交互、工作单元和会话结束事件
│   └── [x] src/ # Agent 生命周期模型
│       └── [x] model.rs # 定义生命周期种类及其关联字段
│           ├── [x] struct TurnLifecycleEvent # 表达 Agent 轮次变化
│           ├── [x] struct UserInteractionEvent # 表达用户交互状态变化
│           └── [x] struct WorkLifecycleEvent # 表达工作单元类型及运行状态
├── [x] process/ # 进程树读取与遍历契约
│   └── [x] tree_snapshot/ # 捕获进程快照并按指定顺序遍历
│       └── [x] src/ # 进程快照模型和查询接口
│           ├── [x] snapshot.rs # 定义树、进程和快照获取入口
│           │   ├── [x] struct TreeSnapshot # 保存同次采样的进程树
│           │   └── [x] trait ProcessTreeSnapshotter # 提供进程树快照
│           └── [x] traversal.rs # 定义快照遍历顺序
│               └── [x] trait SnapshotTraversal # 按指定顺序访问树中的进程
├── [x] provider_label/ # 依据观测证据识别模型服务提供方
│   ├── [x] evidence/ # 分类输入证据
│   │   └── [x] src/ # 证据集合模型
│   │       └── [x] lib.rs # 汇集 provider 判定所需证据
│   │           └── [x] struct EvidenceBundle # 保存供分类器使用的观测证据
│   └── [x] label/ # 分类结果与分类器接口
│       └── [x] src/ # 提供方标签模型
│           └── [x] lib.rs # 定义标签、置信度和分类入口
│               ├── [x] struct ProviderLabelRecord # 保存 provider 名称、置信度和依据
│               └── [x] trait ProviderClassifier # classify 将证据转换为提供方标签
├── [x] semantic_action/ # 语义行为、证据关系、正文和请求轨迹的共享模型
│   └── [x] src/ # 语义数据、解析扩展和持久化端口
│       ├── [x] attr_keys.rs # 定义语义行为属性键。
│       ├── [x] evidence_roles.rs # 定义语义证据角色。
│       ├── [x] link_roles.rs # 定义语义关系角色。
│       ├── [x] model.rs # 定义行为、链接、证据及正文记录
│       │   ├── [x] struct SemanticAction # 表达行为身份、类型、时序、状态和证据
│       │   ├── [x] struct SemanticActionLink # 表达行为间关系、来源与角色
│       │   ├── [x] struct LlmRequestContentWrite # 携带 LLM 请求正文写入数据
│       │   └── [x] struct LlmRequestLineageWrite # 携带请求轨迹延续、分叉及起点依据
│       ├── [x] model/ # 语义种类及行为关联规则。
│       ├── [x] update.rs # 定义已建立行为的增量变化、终态原因和属性补充
│       │   ├── [x] struct SemanticActionUpdate # 携带一次行为更新
│       │   └── [x] enum SemanticActionChange # 区分语义字段与生命周期变化
│       ├── [x] llm.rs # 定义模型响应解析和流式消费接口
│       │   ├── [x] trait LlmProviderResponseParser # 解析 JSON 响应并创建流式响应解析器
│       │   ├── [x] trait LlmProviderResponseStreamParser # 增量处理模型响应事件
│       │   └── [x] struct LlmParsedResponse # 保存文本、工具调用和 token 用量结果
│       ├── [x] content_node.rs # 表达按节点读取的嵌套正文
│       │   ├── [x] enum ContentNode # 区分正文节点类型
│       │   └── [x] struct ContentNodePage # 携带节点分页结果
│       └── [x] store.rs # 定义语义行为及附属内容的存取能力
│           ├── [x] trait SemanticActionWriteStore # 写入行为、链接及正文关联
│           └── [x] trait SemanticActionReadStore # 查询行为、链接及正文
├── [x] graph/ # 可导出的图节点、边和完整性模型
│   └── [x] src/ # 图文档与关系定义
│       ├── [x] document.rs # 表达导出图内容
│       │   └── [x] struct GraphDocument # 聚合 GraphNode 与 GraphEdge
│       └── [x] relationships.rs # 图关系类型
│           └── [x] enum GraphRelationship # 区分图边表达的关系
├── [x] policy/ # 策略评估输入和决策接口
│   └── [x] evaluate/ # 根据观测与上下文产生策略决策
│       └── [x] src/ # 策略输入、评估能力和结果
│           ├── [x] evaluate.rs # 定义评估入口
│           │   └── [x] trait PolicyEvaluator # 将 PolicyInput 转换为策略结果
│           └── [x] decision.rs # 策略输出模型
│               └── [x] struct PolicyDecision # 携带策略决策和相关依据
├── [x] store/ # 按读写、事务、快照和保留策略拆分存储能力
│   ├── [x] read/ # trace、原始事件、payload 与诊断查询
│   │   └── [x] src/ # 各类记录的读取端口
│   │       ├── [x] traces.rs # trace 查询接口
│   │       │   └── [x] trait TraceReadStore # 读取 trace 记录
│   │       ├── [x] events.rs # 原始事件查询接口
│   │       │   └── [x] trait EventReadStore # 读取事实事件
│   │       └── [x] payloads.rs # 明文段过滤与预算查询
│   │           └── [x] trait PayloadReadStore # 按 PayloadSegmentQuery 读取明文段
│   ├── [x] write/ # trace、membership、事件、payload 与诊断写入
│   │   └── [x] src/ # 各类记录的写入端口
│   │       ├── [x] traces.rs # trace 状态写入接口
│   │       │   └── [x] trait TraceWriteStore # 保存 trace 记录
│   │       ├── [x] memberships.rs # trace 成员关系写入接口
│   │       │   └── [x] trait MembershipWriteStore # 保存进程与 trace 成员关系
│   │       └── [x] events.rs # 原始事件写入接口
│   │           └── [x] trait EventWriteStore # 保存事实事件
│   ├── [x] tx/ # 可独立提交或回滚的存储事务
│   │   └── [x] src/ # 事务边界接口
│   │       └── [x] boundary.rs # 定义事务创建与结束能力
│   │           ├── [x] trait TransactionBoundary # 创建写入事务
│   │           └── [x] trait StorageTransaction # 提交或回滚事务
│   ├── [x] snapshot/ # 读取一致快照及控制 trace 租约
│   │   └── [x] src/ # 快照视图与租约模型
│   │       ├── [x] view.rs # 导出快照读取接口
│   │       │   └── [x] trait SnapshotStore # 建立和读取 SnapshotView
│   │       └── [x] lease.rs # 快照期间的 trace 保留租约
│   │           └── [x] trait SnapshotLeaseStore # 管理 TraceLease 的获取与释放
│   └── [x] retention/ # trace 过期候选、删除与墓碑
│       └── [x] src/ # 清理能力与删除记录
│           └── [x] cleanup.rs # 定义保留策略执行入口
│               └── [x] trait RetentionStore # 查询并清理符合条件的 trace
├── [x] alert/ # 插件产生的通用告警定义和读写能力
│   └── [x] src/ # 告警模型、存储端口与错误
│       ├── [x] model.rs # 告警定义、草稿、记录及严重程度
│       │   ├── [x] struct AlertDefinition # 定义告警类型和说明
│       │   └── [x] struct AlertRecord # 保存已提交告警
│       └── [x] store.rs # 告警定义管理、写入与读取接口
│           ├── [x] trait AlertWriteStore # 提交告警记录
│           └── [x] trait AlertReadStore # 查询告警记录
├── [x] alert_delivery/ # daemon→代理的内部协议与外部订阅协议
│   └── [x] src/ # 告警传输模型与有界帧编解码
│       ├── [x] alert.rs # 统一转发告警及 Brain-side、Hand-side 来源
│       │   └── [x] struct ForwardAlert # 携带类别、严重程度、内容和来源
│       ├── [x] internal/ # 生产方握手、心跳和告警传输
│       │   ├── [x] codec.rs # ATAP 内部帧编解码与大小限制
│       │   │   └── [x] struct AtapCodec # 编码和解码内部协议消息
│       │   └── [x] stream.rs # 从连续字节恢复完整内部帧
│       │       └── [x] struct AtapStreamDecoder # 保留未完整接收的帧内容
│       └── [x] subscriber/ # 外部订阅客户端的握手与过滤请求
│           ├── [x] codec.rs # 长度限定 JSON 帧编码与增量解码
│           │   └── [x] struct JsonFrameDecoder # 从输入流中提取 JSON 请求帧
│           └── [x] model.rs # 定义握手、订阅、心跳与外部告警
│               └── [x] struct SubscriptionFilter # 描述订阅的类别与严重程度过滤条件
├── [x] sandbox_control/ # actrail-sb 与 Guest CLI 的连接、状态查询和拒绝结果契约
│   └── [x] src/ # Guest 本地控制请求与端口
│       ├── [x] command.rs # 定义连接与管理命令
│       │   └── [x] enum SandboxControlCommand # 表达 Guest 本地控制操作
│       ├── [x] port.rs # 本地管理服务接口
│       │   └── [x] trait SandboxControlPort # 处理 Guest 控制命令并返回响应
│       └── [x] status.rs # Guest daemon 与上游连接状态
│           └── [x] struct SandboxControlStatus # 汇集 daemon 和连接状态
├── [x] sandbox_link/ # Guest→gateway 与 gateway→daemon 的有界帧协议
│   ├── [x] vsock/ # Guest→gateway 握手、观测批次和流解码
│   │   └── [x] src/ # Guest 链路帧及批次编码
│   │       ├── [x] batch_codec.rs # 观测批次的紧凑编码
│   │       │   └── [x] struct ObservationBatchCodec # 编码和解码 ObservationBatch
│   │       ├── [x] frame.rs # 帧头、消息类型和 Guest 欢迎信息
│   │       │   └── [x] struct Frame # 携带单个 Guest 链路协议帧
│   │       └── [x] stream.rs # 按边界恢复完整帧
│   │           └── [x] struct FrameDecoder # 保存待补全帧并执行大小限制
│   └── [x] upstream/ # gateway→daemon 的会话封装与转发帧
│       └── [x] src/ # 上游协议与增量解码
│           ├── [x] frame.rs # 网关欢迎消息和 Guest 帧封装
│           │   └── [x] struct ForwardedSbFrame # 保留转发的 Guest 帧及上游关联信息
│           └── [x] stream.rs # 恢复完整上游帧
│               └── [x] struct FrameDecoder # 对上游字节流执行有界增量解码
├── [x] sandbox_observation/ # Guest 来源的进程 I/O、资源、压力和 OOM 观测
│   └── [x] src/ # 隔离观测批次及领域模型
│       ├── [x] observation.rs # 统一观测种类与批次
│       │   ├── [x] enum Observation # 区分隔离观测的实际载荷
│       │   └── [x] struct ObservationBatch # 聚合一批 Guest 观测
│       ├── [x] process.rs # Guest 启动身份与进程标记
│       │   └── [x] struct ProcessMarker # 表达 Guest 域中的进程身份坐标
│       ├── [x] resource.rs # Guest CPU 与内存采样
│       │   └── [x] struct GuestResourceSnapshot # 保存 Guest 整体资源快照
│       └── [x] workload.rs # cgroup 与容器工作负载采样
│           └── [x] struct WorkloadCgroupResourceSnapshot # 保存工作负载标识和 cgroup 计数
├── [x] sandbox_plugin_delivery/ # 逐 observation 兴趣匹配、路由计划和插件消费
│   └── [x] src/ # 来源、描述符、路由与投递结果
│       ├── [x] matcher.rs # 查询插件对观测的兴趣
│       │   └── [x] trait SandboxPluginIntentMatcher # 根据描述符生成兴趣查询结果
│       ├── [x] route_plan.rs # 记录消费者与注册代际的匹配计划
│       │   └── [x] struct SandboxRoutePlan # 保存观测到消费者的投递关系
│       └── [x] publisher.rs # 传递匹配后的观测批次
│           ├── [x] trait SandboxObservationConsumer # 消费分配给本插件的观测
│           └── [x] trait SandboxPluginPublisher # 按路由计划发布观测并报告结果
├── [x] sandbox_evidence_store/ # 无插件兴趣的隔离观测证据存储
│   └── [x] src/ # 证据批次、来源与读写端口
│       ├── [x] record.rs # 校验无兴趣批次并保留来源
│       │   └── [x] struct NoInterestEvidenceBatch # 携带应存入证据库的观测
│       ├── [x] write.rs # 有界写入接收接口
│       │   └── [x] trait SandboxEvidenceWritePort # 接收证据批次并返回准入结果
│       └── [x] read.rs # 证据查询接口
│           └── [x] trait SandboxEvidenceReadPort # 按来源与条件读取已存证据
└── [x] sandbox_alert_store/ # 插件产生的隔离告警独立存储
    └── [x] src/ # 告警来源、类型、写入与读取端口
        ├── [x] record.rs # 告警来源与载荷模型
        │   └── [x] struct SandboxAlertRecord # 保存单条隔离告警及其观测来源
        ├── [x] write.rs # 有界告警接收接口
        │   └── [x] trait SandboxAlertWritePort # 接收待存储告警
        ├── [x] commit.rs # 告警提交确认接口
        │   └── [x] trait SandboxAlertCommitPort # 对告警提交提供完成边界
        └── [x] read.rs # 告警查询接口
            └── [x] trait SandboxAlertReadPort # 读取已保存的隔离告警
```
