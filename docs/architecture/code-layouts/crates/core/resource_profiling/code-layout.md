状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/resource_profiling/ # 资源观测组件；统一拥有采集、资源模型、存储、告警、配置和运行编排。
├── [x] Cargo.toml # 构建资源观测组件；依赖通用身份模型、Linux 能力及所选存储驱动。
└── [x] src/ # 资源组件的配置、采集、样本模型、持久化和服务编排。
    ├── [x] alerts/ # 资源告警子模块；拥有阈值解析、默认值与资源样本评估。
    │   ├── [x] mod.rs # 声明并导出资源告警配置和评估器。
    │   ├── [x] resource_alert_config.rs # 解析后的可选资源告警阈值。
    │   │   ├── [x] struct AlertDocument # 告警阈值的字符串配置及默认值。
    │   │   └── [x] struct ResourceAlertConfig # 解析后的可选资源告警阈值。
    │   └── [x] resource_alert_evaluator.rs # 评估资源样本的告警状态。
    │       └── [x] struct ResourceAlertEvaluator # 评估资源样本的告警状态。
    ├── [x] collection/ # 资源采集子模块；拥有采集契约、Host/Workload 后端、采样时钟与采集生命周期。
    ├── [x] config/ # 解析资源总开关，组合采集、存储与告警的独立配置。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   └── [x] resource_profiling_config.rs # 解析总开关，将 collection/storage/alerts 子表交给所属模块解析。
    │       ├── [x] struct RawConfig # 接收资源组件原始子表。
    │       └── [x] struct ResourceProfilingConfig # 组合资源组件总开关、采集、存储和告警配置。
    ├── [x] lib.rs # 导出组件入口、配置、资源模型与应用适配所需接口。
    ├── [x] model/ # 资源专属模型；包含指标值、采样来源、覆盖范围与关联信息。
    │   ├── [x] cgroup_v1_binding.rs # 定义 v1 controller 身份、失效与关闭状态。
    │   │   ├── [x] struct CgroupV1Binding # trace 的双 controller 绑定、宿主启动身份与关闭状态。
    │   │   └── [x] struct CgroupV1Boundary # 单个 controller 的挂载点、成员路径与目录身份。
    │   ├── [x] external_cgroup.rs # 定义外部 cgroup 绑定身份及状态。
    │   │   ├── [x] enum ExternalBindingStaleReason # 定义外部 cgroup 绑定身份及状态。
    │   │   ├── [x] enum ExternalBindingState # 定义外部 cgroup 绑定身份及状态。
    │   │   └── [x] struct ExternalCgroupBinding # 保存目标与外部范围的身份关联、核验及失效状态。
    │   ├── [x] host_resource.rs # boot identity、时间、整机 CPU、内存和负载。
    │   │   └── [x] struct HostResourceSample # boot identity、时间、整机 CPU、内存和负载。
    │   ├── [x] mod.rs # 声明并导出资源样本、指标与控制记录。
    │   ├── [x] resource_sample.rs # 定义统一资源样本入口。
    │   │   └── [x] enum ResourceSample # HostResourceSample 或 WorkloadResourceSample。
    │   ├── [x] resource_scope.rs # 定义自建资源范围身份和生命周期状态。
    │   │   ├── [x] enum ResourceScopeLifecycleState # 表示范围准备、运行与关闭等生命周期状态。
    │   │   └── [x] struct TraceResourceScope # 保存 trace 与自建资源范围的关联和恢复所需控制状态。
    │   ├── [x] serde_identity.rs # 序列化通用 trace 与宿主身份，保持资源样本格式由组件拥有。
    │   │   ├── [x] struct BootIdentity # 序列化通用 trace 与宿主身份，保持资源样本格式由组件拥有。
    │   │   └── [x] struct TraceIdentity # 序列化通用 trace 与宿主身份，保持资源样本格式由组件拥有。
    │   └── [x] workload_resource_sample.rs # 定义工作负载样本及独立资源指标类型。
    │       ├── [x] enum ResourceAccountingCoverage # 资源指标对目标工作负载的覆盖范围。
    │       ├── [x] enum ResourceAccountingMethod # 资源指标的 procfs/cgroup v1/v2 来源。
    │       ├── [x] enum ResourceSampleKind # 周期样本或最终样本。
    │       ├── [x] struct MemoryEventCounters # 定义工作负载样本及独立资源指标类型。
    │       ├── [x] struct ResourceMetrics # CPU、内存等指标值、来源、覆盖范围、样本种类及元数据。
    │       └── [x] struct WorkloadResourceSample # 采样时间、trace/进程关联和资源指标。
    ├── [x] service/ # 组件内编排；组装采集、告警和存储，驱动样本流转与采集收尾。
    │   ├── [x] mod.rs # 声明并导出资源组件入口与装配工厂。
    │   ├── [x] resource_completion.rs # 向应用返回控制完成请求；包含 trace 身份、超时和恢复标记。
    │   │   ├── [x] struct ResourceCompletion # 向应用返回控制完成请求；包含 trace 身份、超时和恢复标记。
    │   │   └── [x] struct ResourceCompletionPoll # 向应用返回控制完成请求；包含 trace 身份、超时和恢复标记。
    │   ├── [x] resource_profiling_factory.rs # 组合各子模块 Factory 并注入应用提供的接口。
    │   │   └── [x] struct ResourceProfilingFactory # 消费 ResourceProfilingConfig，组装采集、存储与告警，返回 ResourceProfilingService。
    │   ├── [x] resource_profiling_service.rs # 提供统一资源观测入口，协调 Host/Workload 调度和生命周期。
    │   │   └── [x] struct ResourceProfilingService # 持有采样器和样本接收器；通过目标与持久化端口获取应用事实，委托各子模块执行。
    │   └── [x] resource_sample_sink.rs # 评估资源告警并向存储后端 best-effort 提交样本批次。
    │       └── [x] struct ResourceSampleSink # 依赖 ResourceStorageBackend，处理 Host/Workload 周期与最终样本；局部记录写入结果后推进采集流程。
    └── [x] storage/ # 资源存储子模块；拥有写入 trait、Database 扩展 trait、Factory、远端、本地和 Hybrid 实现。
```
