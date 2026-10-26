状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/gateway_ingest_runtime/ # daemon 侧网关连接准入与批次交付；通过 sink 接入应用的兴趣路由。
└── [x] src/ # 连接生命周期、交付端口与运行计数。
    ├── [x] lib.rs # 导出摄入运行时、连接句柄、sink 和状态快照。
    ├── [x] runtime.rs # 用共享状态维护连接配额、网关 ID 分配及关闭请求。
    │   ├── [x] struct GatewayIngestRuntime # 持有共享 RuntimeState，提供线程安全准入和状态读取。
    │   │   ├── [x] try_open # 检查关闭状态及连接上限，分配非零 gateway ID。
    │   │   └── [x] request_shutdown # 停止新连接准入，由传输层结束已接入连接。
    │   ├── [x] struct GatewayConnection # 拥有一次连接的 gateway ID，释放时归还连接配额。
    │   │   └── [x] deliver # 携带 gateway ID 和 sb ID 调用 sink，并累计成功或失败计数。
    │   └── [x] enum GatewayOpenError # 区分配置非法、配额满、关闭中及 ID 耗尽。
    ├── [x] sink.rs # 由应用实现的 Sandbox 观测交付边界。
    │   ├── [x] trait SandboxObservationSink # 接收来源坐标与 ObservationBatch，返回局部交付结果。
    │   └── [x] struct SinkDeliveryError # 保存交付失败阶段和说明。
    └── [x] status.rs # 输出连接、心跳、批次数量及交付失败统计。
        └── [x] struct GatewayIngestStatus # 运行计数的只读快照，供控制和诊断展示。
```
