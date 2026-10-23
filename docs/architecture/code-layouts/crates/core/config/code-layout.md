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
    │   ├── [x] agent/ # HTTP、WebSocket 和各层语义保留子配置。
    │   ├── [x] application.rs # 应用协议观测的启用条件和解析参数。
    │   ├── [x] access_control/ # 文件、命令和网络操作的控制后端选择。
    │   │   └── [x] config.rs # 校验控制路由并组合 LSM 等后端配置。
    │   │       └── [x] struct AccessControlConfig # 将具体操作映射到已选择的治理机制。
    │   ├── [x] enforcement.rs # 文件执行控制规则路径、默认决策和拦截参数。
    │   ├── [x] command.rs # 命令裁决、参数读取及灰名单决策预算。
    │   ├── [x] network.rs # 网络连接裁决与通知处理预算。
    │   ├── [x] observation/ # 文件观测需求、摘要和正文保留选择。
    │   ├── [x] finalization/ # trace 收尾与 post-trace 任务的周期、容量和超时。
    │   ├── [x] logging/ # 诊断级别与工作负载诊断开关。
    │   └── [x] cluster.rs # reporter 和 center 的路径、传输与保留配置。
    ├── [x] retention.rs # 主存储清理周期、trace 年龄和保护标签配置。
    │   └── [x] struct StorageRetentionConfig # 将历史记录清理范围和单轮预算交给存储服务。
    ├── [x] provider_rules.rs # 模型服务分类规则位置和未知标签配置。
    │   └── [x] struct ProviderRuleSetConfig # 给 provider 适配器提供规则文件和分类默认值。
    └── [x] export.rs # snapshot 导出的格式、压缩及 payload 包含条件。
        └── [x] struct ExportConfig # 描述一次快照导出所需的输出策略。
```
