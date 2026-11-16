状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config/ # 组合操作配置、采集 profile 和 trace 快照，配置值在对应领域模块校验。
└── [x] src/ # 默认配置生成、输入解析与运行时配置值。
    ├── [x] lib.rs # 导出配置领域模块，供应用组合根调用。
    ├── [x] framework.rs # 统一配置读取、反序列化和模型校验入口。
    │   ├── [x] trait ConfigModel # 约束默认值、序列化与 validate，提供 TOML 读写。
    │   ├── [x] trait ConfigSource # 提供原始配置文本，隔离文件和内联来源。
    │   └── [x] struct ConfigLoader # 将 ConfigSource 输入交给指定 ConfigModel 解析。
    ├── [x] capture_profile.rs # 描述采集需求、失败处理和配置快照生成。
    │   └── [x] struct CaptureProfile # 拥有能力请求及策略，供 sensor plan 协商与 launch 模式选择。
    ├── [x] capture_profile/ # 将部署权限组合映射到具体采集 profile。
    │   └── [x] permission.rs # 定义 host eBPF 与 seccomp 组合下的能力选择。
    ├── [x] trace_snapshot.rs # 保存准入时固定的采集与执行配置。
    │   ├── [x] struct CaptureProfileSnapshot # 固定 profile 名和能力请求，隔离运行中外部配置变化。
    │   └── [x] struct TraceExecutionSnapshot # 描述 trace 执行时使用的配置快照。
    ├── [x] daemon.rs # 汇总 daemon 子配置的公开类型、默认常量和解析入口。
    ├── [x] daemon/ # 按运行职责划分 operator、采集、治理及协议配置。
    │   ├── [x] operator.rs # 对外提供 operator 配置加载、初始化、patch 和写出。
    │   │   ├── [x] struct OperatorConfig # 聚合应用启动所需配置；资源子表交给资源模块解析。
    │   │   │   ├── [x] init # 从当前默认文档构造并校验配置。
    │   │   │   └── [x] patch_files # 按顺序合并指定 TOML patch，重新校验最终配置。
    │   │   └── [x] enum StartupPluginFailurePolicy # 指定启动插件失败时终止或继续。
    │   ├── [x] operator/ # 将分层 TOML 文档转换为有效运行配置。
    │   ├── [x] payload.rs # TLS、socket、stdio 和 MCP 采集配置及后端枚举。
    │   │   └── [x] enum PayloadTlsCaptureBackend # 区分 TLS 明文采集机制及其 seccomp 需求。
    │   ├── [x] process.rs # 进程 syscall 通知、采集队列和身份读取限制。
    │   ├── [x] agent.rs # Agent 识别与语义投影配置类型。
    │   ├── [x] agent/ # HTTP、WebSocket、上下文压缩与语义内容保留配置。
    │   │   ├── [x] http1.rs # 定义 HTTP/1 消息解析的缓冲和证据范围上限。
    │   │   │   └── [x] struct Http1ProtocolConfig # 保存消息缓冲容量和证据范围数量，并提供默认值。
    │   │   ├── [x] websocket.rs # 定义 WebSocket 传输保留策略及模型会话容量。
    │   │   │   ├── [x] struct L2WebSocketRetention # 配置连接、帧、消息、解码内容和证据范围的上限。
    │   │   │   └── [x] struct LlmWebSocketConfig # 限制每个连接的会话通道和待处理请求数量。
    │   │   ├── [x] compact.rs # 定义上下文压缩识别配置及默认值。
    │   │   │   ├── [x] struct LlmContextCompactConfig # 配置压缩识别及证据预算。
    │   │   │   └── [x] struct LlmContextCompactPoolConfig # 配置候选与待采用状态的容量及 TTL。
    │   │   └── [x] retention.rs # 根据各语义层开关和内容归属策略决定实际保留内容。
    │   │       └── [x] impl SemanticRetentionConfig # 查询模型调用、SSE、HTTP、HTTP/2 和传输载荷的保留行为。
    │   │           ├── [x] llm_trajectory_enabled # 判断模型调用层和标准请求块是否支持轨迹记录。
    │   │           ├── [x] sse_event_content_for_llm_response # 根据模型响应消费情况选择 SSE 事件内容。
    │   │           ├── [x] http_body_content_for_http_message # 根据模型层消费和内容归属策略选择 HTTP 正文。
    │   │           └── [x] retain_transport_payload_body # 根据高层消费情况决定是否保留传输载荷正文。
    │   ├── [x] application.rs # 应用协议观测的启用条件和解析参数。
    │   ├── [x] access_control/ # 文件、命令和网络操作的控制后端选择。
    │   │   └── [x] config.rs # 校验控制路由并组合 LSM 等后端配置。
    │   │       └── [x] struct AccessControlConfig # 将具体操作映射到已选择的治理机制。
    │   ├── [x] enforcement.rs # 文件执行控制规则路径、默认决策和拦截参数。
    │   ├── [x] command.rs # 命令裁决、参数读取及灰名单决策预算。
    │   ├── [x] network.rs # 网络连接裁决与通知处理预算。
    │   ├── [x] observation/ # 配置文件活动的采集、汇总和内容保留。
    │   │   └── [x] file.rs # 定义文件观测开关、默认值、保留枚举及终端路径匹配。
    │   │       ├── [x] struct FileObservationConfig # 组合文件采集、汇总、元数据、终端、批量读取和目录枚举配置。
    │   │       ├── [x] struct FileIoSummaryConfig # 设置汇总刷新周期及文件、对象条目容量。
    │   │       ├── [x] struct FileCollectionConfig # 选择打开、路径变更、描述符变更及读写采集项。
    │   │       ├── [x] struct FileIoCollectionConfig # 设置读写观测、次数、字节数和错误计数。
    │   │       │   └── [x] enabled # 判断是否启用了任一读写采集项。
    │   │       ├── [x] enum FileMetadataRetention # 选择完整或精简元数据。
    │   │       ├── [x] enum FileRawEventRetention # 选择完整原始事件、仅错误或汇总。
    │   │       │   ├── [x] retains_success # 判断是否保留成功事件。
    │   │       │   └── [x] retains_error # 判断是否保留错误事件。
    │   │       ├── [x] enum FileBulkReadMode # 选择批量读取计数或路径集合。
    │   │       ├── [x] struct FileTtyObservationConfig # 配置终端路径、操作和原始事件保留方式。
    │   │       │   └── [x] matches # 同时匹配已启用的终端路径规则与操作名称。
    │   │       ├── [x] struct FileBulkReadObservationConfig # 配置批量读取模式、原始事件策略及路径集合容量。
    │   │       └── [x] struct FsEnumerateObservationConfig # 配置目录枚举识别阈值、原始事件策略及路径集合容量。
    │   ├── [x] finalization/ # 配置观测结束后的任务调度与收尾预算。
    │   │   └── [x] config.rs # 定义观测后任务的容量、超时参数和默认常量。
    │   │       └── [x] struct PostTraceRuntimeConfig # 限制并行任务、代理队列、每轮请求及准入、执行、关闭等待时间。
    │   ├── [x] logging/ # 诊断级别与工作负载诊断开关。
    │   └── [x] cluster.rs # reporter 和 center 的路径、传输与保留配置。
    ├── [x] retention.rs # 主存储清理周期、trace 年龄和保护标签配置。
    │   └── [x] struct StorageRetentionConfig # 将历史记录清理范围和单轮预算交给存储服务。
    ├── [x] provider_rules.rs # 模型服务分类规则位置和未知标签配置。
    │   └── [x] struct ProviderRuleSetConfig # 给 provider 适配器提供规则文件和分类默认值。
    └── [x] export.rs # snapshot 导出的格式、压缩及 payload 包含条件。
        └── [x] struct ExportConfig # 描述一次快照导出所需的输出策略。
```
