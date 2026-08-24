# 执行隔离设计

- [执行隔离采集与观测通路设计](base-collection-transport.md)：`actrail-sb` 独立 eBPF 读写计数、Guest 资源轮询、多 SB VSOCK→TCP 代理、`actraild` 独立连接线程及插件/独立数据库路由。
- [actrail-sb Daemon 与连接控制设计](actrail-sb-daemon-lifecycle.md)：快照前 daemon 预热、同 binary CLI、异步 Guest-local control、connection generation gate、Session Owner、事件驱动 main 及断连丢弃边界。
- [Sandbox 资源告警通路](sandbox-alert-pipeline.md)：CPU、内存、OOM 和进程 I/O 告警判定、独立 SQLite 持久化、sandbox source 与告警代理外发边界。
- [端到端时序](end-to-end-sequence.md)：Firecracker 主线中的进程启动、双层连接注册、Guest 采集、即时批量发送、插件/独立数据库路由、告警、静默 Heartbeat、重连与 fail-local 行为。
- [编号式运行时序](numbered-runtime-sequence.md)：按实际配置、线程、握手、进程树、eBPF 计数、采样、转发、路由、告警、丢失和停止动作逐项展开完整 case。
- [目标代码布局](../../architecture/code-layout/execution-isolation/target-layout.md)：执行隔离相关目录、文件职责及 C4 Container/Component。
- [代码布局设计约束](../../architecture/code-layout/execution-isolation/design-constraints.md)：依赖、状态、协议、配置、故障、性能、部署和测试边界。
