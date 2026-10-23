状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/plugin_system/ # 定义插件声明、能力授权和调用端口，管理 Sandbox 消费者的注册及路由快照。
└── [x] src/ # 运行契约、manifest 校验和隔离观测交付。
    ├── [x] lib.rs # 汇总宿主、消费者、控制、codec 及声明的公开类型。
    ├── [x] manifest/ # 解析插件用途、运行形式、订阅和资源声明。
    │   ├── [x] declarations.rs # 定义 manifest 文档结构及分角色配置。
    │   │   └── [x] struct PluginManifest # 聚合插件元数据、runtime、host、role 和输出声明。
    │   └── [x] types.rs # 定义插件用途、运行机制和交付模式。
    │       ├── [x] enum PluginPurpose # 区分 observation、control 和 codec 等职责。
    │       └── [x] enum PluginRuntimeKind # 区分内置和 WASM 等实例运行形式。
    ├── [x] grants/ # 解析并校验宿主能力及规则作用域授权。
    │   └── [x] model.rs # 表示可调用 host 能力和受限策略写权限。
    │       └── [x] struct PluginHostGrants # 保存经授权的 hostcall 能力与作用域。
    ├── [x] observation.rs # 实时观测和 post-trace 分析的消费及查询端口。
    │   ├── [x] trait ObservationConsumer # 消费允许发布的语义批次，返回交付与处理结果。
    │   ├── [x] trait PostTraceAnalyzer # 对已结束 trace 执行受限分析任务。
    │   └── [x] struct ObservationBatch # 借用本批动作、关联与 payload 引用，明确调用边界。
    ├── [x] control.rs # 待裁决上下文、决策预算和文件策略宿主端口。
    │   ├── [x] trait ControlDecider # 根据操作上下文在预算内返回治理决定。
    │   ├── [x] struct ControlDecisionRequest # 携带当前决策身份、操作主体和上下文引用。
    │   └── [x] trait FilePolicyHost # 在授权作用域内读取或修改文件策略。
    ├── [x] command_policy/ # 命令执行上下文和命令规则管理契约。
    ├── [x] network_policy.rs # 连接上下文、远端选择器和网络规则管理契约。
    ├── [x] llm_codec.rs # 请求及 SSE 解码插件的输入输出契约。
    ├── [x] alert.rs # 插件向宿主提交声明内告警的能力端口。
    ├── [x] sandbox_observation/ # 根据注册快照匹配 Guest 观测并按消费者队列投递。
    │   ├── [x] registry.rs # 拥有注册表、generation、消费者资源和只读快照。
    │   │   ├── [x] struct SandboxPluginFacade # 提供 register/unregister、matcher 和 publisher 入口。
    │   │   ├── [x] struct SandboxPluginRegistry # 统一修改注册状态并发布新的匹配快照。
    │   │   └── [x] struct SandboxConsumerRegistration # 定义消费者身份、订阅及交付资源。
    │   └── [x] worker.rs # 驱动单消费者批次处理，隔离队列压力和使用方失败。
    ├── [x] runtime.rs # 内置实例与实例 ID 的基础值。
    │   └── [x] struct PluginInstanceId # 标识一次已加载插件实例，用于规则 ownership 和状态查询。
    ├── [x] status.rs # 插件生命周期、hostcall 和运行统计的查询表示。
    └── [x] diagnostics.rs # 插件调用错误和被丢弃记录的诊断表示。
```
