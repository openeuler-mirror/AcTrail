# 执行隔离代码布局

> 本文展示 Guest 采集、VSOCK 传输、网关摄入和 Sandbox 告警的源码归属。

VSOCK 是主机与 Guest 之间的套接字传输。下方箭头表示“该应用组合或依赖此运行时、契约或适配器”。

```text
crates/apps/sb
  -> crates/core/sandbox_agent_runtime
  -> crates/adapters/collectors/sandbox_linux
  -> crates/contracts/sandbox_control
  -> crates/adapters/sandbox_control/uds
  -> crates/contracts/sandbox_link/vsock
  -> crates/adapters/sandbox_link/vsock

crates/apps/vsock_gateway
  -> crates/core/vsock_gateway_runtime
  -> sandbox-link VSOCK 适配器

crates/apps/daemon
  -> crates/core/gateway_ingest_runtime
  -> Sandbox 插件和告警服务
  -> 相互独立的 Sandbox Evidence 和 Alert 存储
```

`crates/apps/sb` 负责 daemon 启动、静态配置、Guest 实例锁、信号与控制健康状态的生命周期，以及 CLI 分派。`sandbox_agent_runtime` 负责工作单元、连接门、端点与会话状态、批处理、心跳和重连行为。

Guest Linux 采集器负责自身的 eBPF 对象、内核 map、挂载 link、进程谱系发现、I/O 聚合、内存不足（OOM）事件队列，以及 Linux procfs 资源采样。它与 brain-side 采集器仅共享通用的标准 tracepoint 挂载适配器；不共享采集状态或生命周期。

`vsock_gateway_runtime` 负责 SB 会话、每会话配额、全局上游队列、网关握手和重连行为。`gateway_ingest_runtime` 负责 daemon 侧连接注册和 hand-side 观测投递。Daemon 的应用级服务执行插件或证据路由，并转发已提交的告警。
