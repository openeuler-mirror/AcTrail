状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/daemon/ # 宿主守护进程：装配采集、存储、治理和插件服务，驱动控制请求与观测事件。
├── [x] Cargo.toml # actraild 二进制与宿主应用依赖。
└── [x] src/ # 守护进程启动、控制循环与具体服务装配。
    ├── [x] lib.rs # 导出守护进程构建入口并声明内部模块。
    ├── [x] bin/ # actraild 命令行与进程管理入口。
    │   ├── [x] actraild.rs # 启动命令行分发并将失败映射到进程退出。
    │   └── [x] actraild/ # 配置加载、日志、信号和后台进程监督。
    │       ├── [x] args.rs # 解析启动、停止、状态和插件命令。
    │       │   ├── [x] enum AcTraildCommand # 表达守护进程管理请求及所需参数。
    │       │   └── [x] enum PluginCommand # 表达插件加载、卸载、配置和命令调用。
    │       ├── [x] entry.rs # 校验配置并执行所选管理命令。
    │       ├── [x] process.rs # 管理 PID、就绪握手和子进程退出。
    │       │   ├── [x] enum DaemonProcessState # 区分 PID 记录对应的守护进程状态。
    │       │   └── [x] struct DaemonStartSupervisor # 借用启动配置，监督后台子进程的就绪与退出。
    │       ├── [x] plugin_registry.rs # 保存启动时需要恢复的插件实例配置。
    │       │   └── [x] struct PersistentPluginRegistry # 持有持久化插件实例清单。
    │       ├── [x] signals.rs # 将终止信号接入守护进程退出流程。
    │       └── [x] logging.rs # 配置日志输出和运行诊断格式。
    ├── [x] startup/ # 启动前解析环境能力并构建依赖。
    │   ├── [x] bootstrap.rs # 将运行配置与采集、存储服务组合为本地控制服务器。
    │   │   ├── [x] struct DaemonBootstrap # 接收 DaemonRuntimeWiring 并构建控制服务器。
    │   │   └── [x] struct LocalDaemonServer # 持有控制服务器与服务宿主，提供请求处理、事件排空和关闭入口。
    │   │       ├── [x] build # 校验配置并构建宿主服务依赖。
    │   │       ├── [x] drain_live_events # 推进采集事件与后台服务。
    │   │       └── [x] shutdown # 驱动服务关闭与最终数据排空。
    │   ├── [x] profiles.rs # 维护采集 profile 和仅适用于 launch 的 profile 集合。
    │   │   └── [x] struct DaemonProfileRegistry # 持有按名称索引的 profile，提供插入与查询。
    │   ├── [x] ebpf_resolve.rs # 解析 eBPF 采集器部署位置与可用能力。
    │   │   └── [x] struct EbpfResolution # 承载 eBPF 启动解析结果。
    │   └── [x] host_id.rs # 获取宿主身份，供 trace 和存储记录关联。
    ├── [x] runtime_wiring.rs # 定义控制宿主接收的运行依赖集合。
    │   └── [x] struct DaemonRuntimeWiring # 转移 TraceRuntime、AttachService 和告警转发插件所有权，并携带容量与就绪信息。
    ├── [x] service_host.rs # 将控制操作绑定到 trace 生命周期和具体接入服务。
    │   ├── [x] trait AttachService # 约束进程接入、事件驱动、策略插件操作与关闭行为。
    │   ├── [x] trait AttachDebugService # 为控制诊断提供采集器状态快照。
    │   └── [x] struct DaemonServiceHost # 持有 TraceRuntime 与接入服务，实施控制请求授权并协调生命周期。
    ├── [x] peer_identity.rs # 从连接对端提取身份并检查进程和 trace 操作权限。
    │   ├── [x] struct PeerPrincipal # 表达授权主体并与 trace owner 比对。
    │   └── [x] struct PeerIdentity # 解析对端凭证，验证 root、进程引用与 trace 所有权。
    ├── [x] control_loop.rs # 推进控制连接的读取、请求处理和响应写出。
    ├── [x] socket_loop.rs # 轮询控制连接、采集 FD 和后台截止时间，驱动整体运行循环。
    │   └── [x] struct DaemonRunError # 保存守护进程运行错误的阶段代码与说明。
    ├── [x] services.rs # 声明具体服务模块并导出运行依赖构建入口。
    └── [x] services/ # 将核心组件和系统适配器接入守护进程生命周期。
```
