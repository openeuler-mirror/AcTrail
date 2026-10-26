状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/plugin_wasm_runtime/ # 将插件系统端口适配到 WASM core module 和 component，执行资源及 hostcall 限制。
└── [x] src/ # 引擎状态、角色实例、内存检查和授权宿主调用。
    ├── [x] lib.rs # 导出 observation、control、LLM codec 的构造入口与适配器。
    ├── [x] engine.rs # 构造带 fuel 的引擎及有限 Store，转换预算与运行错误。
    │   ├── [x] type WasmStore # 使用 WasmStoreState 的 wasmtime Store，统一承载宿主上下文。
    │   └── [x] struct WasmHostcallMetrics # 累计受限宿主调用的字节、次数及失败状态。
    ├── [x] engine/ # 单实例 Store 中的授权和调用上下文。
    │   └── [x] state.rs # 保存内存限制、host grants、配置和当前观测或控制上下文。
    │       ├── [x] struct WasmStoreState # 拥有本 Store 的宿主句柄与可访问上下文，随调用设置和清理。
    │       ├── [x] struct WasmHostLimits # 约束正文、配置、操作上下文和命令输出大小。
    │       └── [x] struct PostTraceTaskContext # 保存当前分析 trace 和累积查询预算。
    ├── [x] observation.rs # 选择 legacy 或 component 观测实例并实现统一消费接口。
    │   └── [x] struct WasmObservationConsumer # 将 observation 批次和分析任务交给所选 WASM 实现。
    ├── [x] control.rs # 创建控制实例、限制并发并设置单次调用时限。
    │   ├── [x] struct WasmControlDecider # 对外实现 control-decider，封装不同 WASM ABI。
    │   └── [x] struct EpochDeadline # 关联调用 deadline 与 epoch 中断，限制超时执行。
    ├── [x] codec.rs # 用 WASM codec 扩展 provider 请求和流式事件解析。
    │   └── [x] struct WasmLlmCodecPlugin # 拥有 codec 实例与调用状态，返回标准化解析结果。
    ├── [x] component_control.rs # component ABI 的决策调用、宿主连接和运行状态。
    │   └── [x] struct WitComponentControlDecider # 承载 WIT 控制接口并适配当前操作上下文。
    ├── [x] component_control/ # component 的命令、网络及配置 hostcall 编解码。
    │   ├── [x] runtime_config.rs # 查找并调用 component 的运行配置接口。
    │   └── [x] network/ # 网络控制上下文和规则宿主适配。
    ├── [x] component_observation/ # component 观测消费、告警提交和 post-trace 查询。
    │   ├── [x] consumer.rs # 拥有 component 观测实例并处理消费生命周期。
    │   │   └── [x] struct WitComponentObservationConsumer # 在预算内调用观测与收尾分析接口。
    │   ├── [x] host.rs # 按 grants 访问上下文、payload 与配置。
    │   ├── [x] alert_host.rs # 校验告警授权与输入后调用宿主告警端口。
    │   └── [x] post_trace_host.rs # 执行受限 action、activity、file-state 分页查询。
    ├── [x] host/ # legacy ABI 的文件、命令、网络和数据读取 hostcall。
    │   ├── [x] data_access.rs # 限制 payload、环境和配置读取的来源及字节数。
    │   ├── [x] file_policy.rs # 将受限二进制输入转换为文件规则操作。
    │   └── [x] command/ # 编解码命令策略和当前执行上下文。
    └── [x] memory.rs # 验证 WASM 线性内存区间并受控读写调用数据。
```
