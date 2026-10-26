状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/model/src/ # 为各运行时提供共享记录和类型化身份；可变状态由对应服务持有。
├── [x] lib.rs # 导出领域模型模块。
├── [x] ids.rs # 区分本地记录 ID、OTel trace 身份及显示名称。
│   ├── [x] struct OtelTraceId # 固定 16 字节导出身份，校验非零值。
│   └── [x] struct TraceName # trace 的显示名称，不替代 TraceId 作为关联键。
├── [x] process.rs # 复用稳定进程身份类型并定义 trace 内 membership。
│   ├── [x] struct ProcessMembership # 关联 trace、进程、父身份、采集及传播开关。
│   ├── [x] enum MembershipState # 表达进程归属的启动、活跃和退出阶段。
│   └── [x] struct ExitStatus # 保存退出码、信号和观测时间。
├── [x] trace.rs # trace 元数据、生命周期、健康状态和时间记录。
│   ├── [x] struct TraceRecord # 聚合根身份、名称、profile、标签、生命周期和导出身份。
│   ├── [x] enum TraceLifecycleState # 表达 Starting、Active、Draining 及终态。
│   └── [x] enum TraceHealth # 将观测降级与 trace 生命周期分开表示。
├── [x] event.rs # 携带规范化身份及来源信息的领域事件。
│   ├── [x] struct EventEnvelope # 关联事件 ID、trace、进程和观测时间。
│   ├── [x] enum EventPayload # 区分进程、文件、网络、IPC、协议、治理和丢失等事件内容。
│   └── [x] struct DomainEvent # 组合事件 envelope、payload 和记录策略事实。
├── [x] payload/ # 明文段、传输来源和内容保留状态。
│   ├── [x] mod.rs # 导出载荷身份与段模型。
│   └── [x] segment.rs # 定义段、操作、流及内容状态，供采集和解析共享。
│       ├── [x] struct PayloadSegment # 保存来源边界、流坐标、字节和采集完整性。
│       └── [x] struct PayloadStreamIdentity # 将段归并到明确来源和进程代际的逻辑流。
├── [x] capability.rs # 用能力及字段保证描述采集需求和供给。
│   ├── [x] enum Capability # 标识具体观测或治理能力。
│   ├── [x] enum RequestMode # 区分必需、尽力和禁用的能力请求。
│   └── [x] struct CapabilityDescriptor # 记录能力及各字段可提供的保证等级。
├── [x] diagnostics.rs # 运行异常、丢弃、覆盖不足及 LLM 阶段诊断。
│   ├── [x] struct DiagnosticRecord # 关联诊断 ID、trace、严重性与元数据。
│   └── [x] struct LlmPipelineDiagnostic # 保留 LLM 解析阶段、诊断码和来源上下文。
├── [x] policy.rs # 记录观测处理中的允许、丢弃、截断和脱敏事实。
│   └── [x] struct PolicyRecord # 将策略 verdict、截断与脱敏结果附加到观测。
├── [x] binary_identity.rs # 表示探针计划引用的二进制身份类型及内容。
│   └── [x] struct BinaryIdentity # 约束二进制身份表示，用于计划匹配和校验。
├── [x] container.rs # 规范容器运行时和完整容器标识。
│   └── [x] struct NormalizedContainerId # 验证并保存完整容器 ID，避免短前缀关联。
└── [x] host_identity.rs # 表示宿主本次启动身份。
    └── [x] struct HostBootId # 固定 boot ID 表示，隔离不同启动周期的资源记录。
```
