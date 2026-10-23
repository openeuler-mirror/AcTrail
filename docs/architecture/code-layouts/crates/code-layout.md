状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/ # Rust 工作空间；按应用、领域运行时、契约与平台适配划分职责
├── [x] apps/ # 命令行与服务端入口；装配领域运行时和平台实现
│   ├── [x] daemon/ # actraild 常驻观测服务；启动采集器、管理观测任务并处理控制请求
│   │   └── [x] src/ # daemon 启动与服务装配源码
│   │       └── [x] services/ # 连接 trace、采集、存储与控制的应用服务
│   │           └── [x] resource_profiling/ # 资源组件的应用适配；提供 trace/进程事实、生命周期通知与控制状态持久化。
│   ├── [x] ctl/ # actrailctl 命令行控制入口；生成配置、启动或接管目标进程并监督运行
│   ├── [x] view/ # actrailviewer；读取本地观测记录，按条件查询并导出结果
│   │   └── [x] src/ # 命令行查询、过滤与结果输出
│   ├── [x] web/ # actrailweb；本地/集群查询与受支持的控制操作
│   │   ├── [x] frontend/ # 浏览器页面组件、查询交互与界面状态
│   │   └── [x] src/ # HTTP 服务、查询后端与控制请求处理
│   │       └── [x] http/ # Web 路由与 HTTP 请求到查询服务的适配
│   ├── [x] cluster/ # reporter 与 center；bundle、索引及上报状态
│   ├── [x] sb/ # actrail-sb；Guest 采集服务组合
│   ├── [x] vsock_gateway/ # Guest VSOCK 到 daemon TCP 的有界转发
│   └── [x] alert_proxy/ # 独立告警代理与订阅连接
├── [x] core/ # 领域模型、配置与运行时
│   ├── [x] config/ # operator 配置组合、默认值与校验
│   │   └── [x] src/ # 按运行角色解析配置、补全默认值并校验
│   │       └── [x] daemon/ # daemon 启动与运行参数
│   │           └── [x] operator/ # 运维者配置；组合各领域配置组
│   ├── [x] model/ # 通用身份与领域值
│   │   └── [x] src/ # 跨领域共享的身份、事件与查询值类型
│   ├── [x] process_identity/ # 为进程建立稳定身份，关联不同 PID 坐标，避免 PID 复用混淆观测
│   ├── [x] trace_runtime/ # 管理一次观测任务的生命周期、所属进程与采集计划
│   ├── [x] ingest_runtime/ # 接收采集事件，解析进程与观测任务归属，再交给记录和语义处理
│   ├── [x] semantic_action_runtime/ # 从网络与进程活动还原 LLM 请求、工具调用等可查询的语义动作
│   │   └── [x] src/ # 将观测字节与协议消息投影为语义动作
│   │       └── [x] llm_pipeline/ # LLM 请求响应识别、解码与语义投影
│   │           └── [x] transport/ # 从传输流还原 HTTP/WebSocket 消息边界
│   │               └── [x] http1/ # HTTP/1 消息边界与内容解码
│   ├── [x] plugin_wasm_runtime/ # 加载并执行 WASM 插件，限制调用预算并提供宿主能力
│   ├── [x] resource_profiling/ # 采集主机和工作负载的 CPU、内存等资源指标，负责存储与告警
│   ├── [x] sandbox_agent_runtime/ # 管理隔离环境内的采集工作单元、周期采样与上报连接
│   ├── [x] vsock_gateway_runtime/ # 管理 Guest 转发会话，限制连接资源并缓存等待发送的数据
│   └── [x] gateway_ingest_runtime/ # daemon 接收隔离环境的观测上报，并接入本地事件处理链
├── [x] contracts/ # 跨组件的数据类型与端口
│   ├── [x] sandbox_control/ # actrail-sb 与 Guest CLI 的连接、状态查询和拒绝结果契约
│   └── [x] sandbox_link/ # Guest 到 gateway、gateway 到 daemon 的有界帧协议
│       └── [x] vsock/ # Guest 到 gateway 的握手、观测批次编码与增量帧解码
├── [x] adapters/ # 内核采集、控制传输、隔离链路和 Sandbox 存储
│   ├── [x] collectors/ # 将 Linux 内核及 procfs 信号转换为观测事件
│   │   ├── [x] ebpf/ # 挂载 BPF 程序、维护进程采集绑定并解码环形缓冲
│   │   │   └── [x] bpf/ # 内核采集的 ABI、运行时与领域模块
│   │   └── [x] sandbox_linux/ # Guest 进程 I/O、资源与 cgroup 采集
│   ├── [x] sandbox_control/ # Guest 本地管理传输
│   │   └── [x] uds/ # Guest 控制命令的 Unix socket 客户端、服务端与编解码
│   └── [x] sandbox_link/ # Guest 与 gateway 连接及 daemon 上游监听
│       └── [x] vsock/ # 内核 VSOCK 与 Unix stream 连接适配
├── [x] storage/ # 主存储接口、Factory 与 SQLite/noop 实现
├── [x] recording/ # 观测记录与语义发布的交付协调
│   └── [x] runtime/ # 主存储事务与语义发布协调，分别记录交付结果
├── [x] export/ # 在线导出运行时与 OTEL HTTP/JSONL 适配器
├── [x] access_control/ # 访问规则与 LSM 控制能力
├── [x] plugin/ # WASM 插件与宿主共享的调用接口
│   └── [x] abi/ # 插件 ABI
├── [x] plugins/ # 内置插件与 WASM 实现
└── [x] tools/ # 探针发现、TLS 与存储诊断工具
```
