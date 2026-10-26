状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/vsock_gateway_runtime/ # 将多个 Guest 的 VSOCK 连接转发到 daemon TCP 上游，限制会话与待发送帧。
└── [x] src/ # 网关生命周期、连接注册与上游发送队列。
    ├── [x] lib.rs # 导出网关配置、运行时和状态快照。
    ├── [x] config.rs # 校验监听、上游、帧长度、连接和线程资源限制。
    │   └── [x] struct GatewayConfig # 持有 VSOCK 监听、TCP 上游及转发预算。
    ├── [x] runtime.rs # 建立上游、监听 Guest 并管理接收线程及关闭流程。
    │   ├── [x] struct GatewayRuntime # 拥有会话注册表、上游链路、停止信号和 accept 线程。
    │   │   ├── [x] start # 校验配置并建立上游和监听，启动连接处理。
    │   │   └── [x] shutdown # 通知线程退出，停止接收并回收上游运行资源。
    │   ├── [x] struct SbConnectionWorker # 处理单个 Guest 的握手、帧校验和转发。
    │   └── [x] struct GatewaySnapshot # 展示当前 gateway ID、活跃 Guest 数和转发计数。
    ├── [x] session.rs # 将 Guest 会话接入与释放集中在同一注册表。
    │   └── [x] struct SessionRegistry # 在容量限制内分配和释放 sb 会话身份。
    └── [x] upstream.rs # 维护 TCP 上游与有界发送工作线程。
        ├── [x] struct UpstreamLink # 拥有上游身份、发送端和工作线程的生命周期。
        ├── [x] struct UpstreamSender # 将带会话来源的帧提交到发送队列。
        ├── [x] struct SessionForwardQuota # 限制单 Guest 尚未发送的转发工作量。
        └── [x] struct SessionForwardPermit # 随队列项生命周期归还会话额度。
```
