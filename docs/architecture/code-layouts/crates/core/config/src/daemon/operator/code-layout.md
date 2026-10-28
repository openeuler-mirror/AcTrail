状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config/src/daemon/operator/ # 解析分层 operator 文档，按所属领域生成默认值并执行条件校验。
├── [x] document.rs # 组合全部顶层配置组，资源子表委托 resource_profiling。
│   └── [x] struct OperatorDocument # 保存外部 TOML 文档，转换为应用使用的 OperatorConfig。
│       ├── [x] default_toml # 汇总各文档默认值并注入资源模块的默认子树。
│       └── [x] to_config # 将字符串、路径和各组文档转换成有效运行配置。
└── [x] document/ # 子模块拥有本组字段、默认值及转换约束。
    ├── [x] base/ # 控制端点、主存储、Web、capture 与 eBPF 基础配置。
    │   ├── [x] document.rs # 定义基础配置文档并执行路径、预算和枚举校验。
    │   │   ├── [x] struct ControlDocument # 控制 socket、trace 容量、日志和 finalization 入口。
    │   │   ├── [x] struct StorageDocument # 选择主存储后端和 retention 策略。
    │   │   └── [x] struct EbpfDocument # 解释 enabled 模式及 eBPF 采集参数。
    │   └── [x] ipc_lineage.rs # 配置 IPC 关系观察的范围及容量。
    ├── [x] payload.rs # 转换 TLS、socket、stdio 和 MCP 的明文采集及保留参数。
    ├── [x] payload/ # TLS 等载荷文档共享默认常量。
    │   └── [x] constant.rs # 集中定义默认 TLS capture backend。
    ├── [x] semantic.rs # 汇总语义层保留文档及投影开关。
    ├── [x] semantic/ # LLM、HTTP 和 WebSocket 的内容及状态预算。
    │   ├── [x] llm.rs # 请求正文、工具结果、trajectory 和 assembly 的保留参数。
    │   ├── [x] compact.rs # 解析并校验上下文压缩配置。
    │   ├── [x] http.rs # HTTP、SSE、HTTP/2 的语义和正文保留参数。
    │   └── [x] websocket.rs # WebSocket 消息和语义内容的保留参数。
    ├── [x] plugin/ # 插件发现、告警队列和启动加载文档。
    │   └── [x] config.rs # 解释插件配置以及全局和逐项 startup failure policy。
    │       └── [x] struct StartupPluginsDocument # 保存启动加载开关、失败策略与插件列表。
    ├── [x] alert_forwarding/ # 告警代理地址、转发插件和投递预算文档。
    ├── [x] hand_observation/ # daemon 隔离观测 TCP 入口及连接限制。
    ├── [x] sandbox_evidence/ # 无兴趣 observation 的 SQLite 路径、写队列及保留策略。
    ├── [x] sandbox_alerts/ # Sandbox 告警独立存储和读写预算。
    ├── [x] process.rs # seccomp 通知、进程观测和 agent invocation 文档。
    ├── [x] file.rs # 文件事件采集需求、路径和 IO 摘要配置。
    ├── [x] idle_detection.rs # 将 idle detector 子配置交给领域模块校验。
    ├── [x] command.rs # 命令控制规则及异步决策容量文档。
    ├── [x] network.rs # 网络控制规则及 syscall 选择文档。
    ├── [x] app.rs # 应用协议、provider、文件治理及启停监督文档。
    ├── [x] cluster.rs # 集群上报、中心与目录配置文档。
    └── [x] helpers.rs # 复用正数、时长、路径和枚举输入校验。
```
