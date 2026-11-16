状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/ # Rust 工作空间；按应用、领域运行时、契约与平台适配划分职责
├── [x] apps/ # 命令行与服务端入口；装配领域运行时和平台实现
│   ├── [x] daemon/ # actraild 常驻观测服务；启动采集器、管理观测任务并处理控制请求
│   │   └── [x] src/ # daemon 启动与服务装配源码
│   │       └── [x] services/ # 连接 trace、采集、存储与控制的应用服务
│   │           └── [x] resource_profiling/ # 资源组件的应用适配；提供 trace/进程事实、生命周期通知与控制状态持久化。
│   ├── [x] ctl/ # actrailctl 命令行控制入口；生成配置、启动或接管目标进程并监督运行
│   ├── [x] view/ # 从本地存储读取 trace、语义动作和资源指标，输出终端视图或导出文件。
│   │   ├── [x] src/ # 解析查询命令、选择数据源并渲染结果。
│   │   └── [x] Cargo.toml # actrailviewer 二进制、存储接口和导出器依赖。
│   ├── [x] web/ # actrailweb；本地/集群查询与受支持的控制操作
│   │   ├── [x] frontend/ # 浏览器页面组件、查询交互与界面状态
│   │   └── [x] src/ # HTTP 服务、查询后端与控制请求处理
│   │       └── [x] http/ # Web 路由与 HTTP 请求到查询服务的适配
│   ├── [x] cluster/ # actrailcluster；本地 trace 打包上报与中心接收索引的应用组合
│   │   ├── [x] Cargo.toml # 配置、存储、JSON graph 导出与 SQLite 索引依赖
│   │   └── [x] src/ # bundle 编解码、HTTP 上传接收和周期上报
│   │       ├── [x] bin/ # 集群命令可执行入口
│   │       │   └── [x] actrailcluster.rs # 将参数交给集群应用并返回执行结果
│   │       ├── [x] lib.rs # 分派 pack/upload/serve/import/list；装配本地只读存储与中心索引
│   │       │   ├── [x] struct Cli # 保存 operator 配置路径与子命令
│   │       │   ├── [x] enum Command # 区分单次打包、单次或循环上传、中心监听及离线导入查询
│   │       │   ├── [x] struct BundleDocument # 拥有 manifest、JSON graph 和可选 SQLite snapshot 字节
│   │       │   ├── [x] struct BundleManifest # 记录 bundle 格式、生成时间、节点身份、trace 信息和计数
│   │       │   ├── [x] struct BundleTrace # 同时保存外部 trace_uid 与本地 local_trace_id 及生命周期摘要
│   │       │   ├── [x] struct BundleCluster # 保存集群和节点的标识、名称与地址
│   │       │   ├── [x] struct BundleCounts # 汇总事件、动作、关联、payload 与诊断数量
│   │       │   ├── [x] struct ReportState # 独立 SQLite 连接持有各 trace 的上传状态、指纹和失败信息
│   │       │   │   ├── [x] open # 准备上报状态表并读取已有上传记录
│   │       │   │   └── [x] mark_uploaded # 在上传成功后保存 bundle 指纹和服务端回复
│   │       │   ├── [x] struct UploadSummary # 返回上传成功、未改变跳过及失败数量
│   │       │   └── [x] struct HttpRequest # 接收中心请求的方法、路径、请求头和正文
│   │       └── [x] upload_loop.rs # 按配置周期重载并执行 upload_once，处理信号和实例互斥
│   │           └── [x] struct UploadLoopLock # 持有循环上报的进程锁，释放时结束该实例的独占权
│   ├── [x] sb/ # actrail-sb；Guest 采集服务组合
│   ├── [x] vsock_gateway/ # Guest VSOCK 到 daemon TCP 的有界转发
│   ├── [x] grey_proxy/ # WASM grey 运行时的子进程，与所属运行时同生同死；补充近期进程动作并请求外部裁决
│   │   ├── [x] Cargo.toml # 子进程二进制入口；复用现有存储查询与 HTTP 能力
│   │   └── [x] src/ # 待决动作来自 WASM 请求，逻辑进程 ID 来自 daemon，历史事实来自现有数据库
│   │       ├── [x] bin/ # 子进程入口
│   │       │   └── [x] actraild-grey-proxy.rs # 接收运行时提供的配置，启动请求处理，并随所属运行时退出
│   │       ├── [x] config.rs # 外部裁决服务地址、历史时间窗口和数量上限；存储位置及已有运行限制由宿主配置提供
│   │       │   └── [x] struct Config # 解析并持有 proxy 所需配置，整体传入请求处理对象
│   │       ├── [x] proxy.rs # 接收待决请求，补充历史，调用外部服务并返回结果
│   │       │   └── [x] struct GreyProxy # 组织单次处理；历史不可用时注明原因，裁决调用失败交回现有 grey 故障处理
│   │       ├── [x] history.rs # 倒序选取有界历史 ID，再批量读取选中记录的属性
│   │       │   └── [x] struct HistoryReader # 按 trace、逻辑进程 ID 和时间窗口提取访问裁决及执行命令，输出精简事实
│   │       └── [x] remote.rs # 外部裁决服务调用
│   │           └── [x] struct DecisionClient # 发送待决动作与历史上下文，返回外部裁决；沿用现有调用预算和资源限制
│   └── [x] alert_proxy/ # 独立告警代理与订阅连接
├── [x] core/ # 领域模型、配置与运行时
│   ├── [x] config/ # operator 配置组合、默认值与校验
│   │   └── [x] src/ # 按运行角色解析配置、补全默认值并校验
│   │       └── [x] daemon/ # daemon 启动与运行参数
│   │           └── [x] operator/ # 运维者配置；组合各领域配置组
│   ├── [x] config_export_filter/ # 组合所属模块模板与交付选择策略，供 CLI 导出和 Web 编辑共用
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
│   ├── [x] checkpoint/ # 协调文件状态的保存与恢复，负责 scope 规划和组装 backend
│   ├── [x] sandbox_agent_runtime/ # 管理隔离环境内的采集工作单元、周期采样与上报连接
│   ├── [x] vsock_gateway_runtime/ # 管理 Guest 转发会话，限制连接资源并缓存等待发送的数据
│   └── [x] gateway_ingest_runtime/ # daemon 接收隔离环境的观测上报，并接入本地事件处理链
├── [x] contracts/ # 跨组件的数据类型与端口
│   ├── [x] checkpoint/ # 同步 abi_stable 契约；core 与集成动态库共用接口和类型
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
├── [x] plugin/ # 宿主与插件共享的底层 ABI 常量
│   └── [x] abi/ # 可在 no_std 环境使用的控制协议标识
│       └── [x] src/ # ABI 常量定义与模块导出
│           ├── [x] lib.rs # 启用 no_std 并导出 control 模块
│           └── [x] control.rs # 定义上下文句柄、查询名称、能力授权名称和二进制操作码
│               ├── [x] mod context # 当前文件、命令和网络控制上下文句柄
│               ├── [x] mod query # 决策摘要、规则和操作上下文的查询标识
│               ├── [x] mod file_policy # 文件裁决、规则更新、应用模式及授权标识
│               ├── [x] mod command_policy # 命令规则协议版本、裁决与更新操作码及授权标识
│               ├── [x] mod network_policy # 网络规则协议版本、裁决与更新操作码及授权标识
│               └── [x] mod subject_code # 文件、命令和网络裁决对象的类型码
├── [x] plugins/ # 内置插件与 WASM 插件实现
│   ├── [x] grey_forwarding/ # WASM 控制插件；使用运行时提供的连接信息，将待决动作、trace 和逻辑进程 ID 发送至 grey_proxy，返回裁决或调用失败
│   ├── [x] alert_forwarding/ # 依据类别过滤告警，向当前连接提供非阻塞投递入口
│   │   └── [x] src/ # 转发配置、连接代际与投递状态
│   │       ├── [x] lib.rs # 导出配置、插件运行时和投递结果
│   │       ├── [x] runtime.rs # 管理当前转发连接、启停状态和丢弃计数
│   │       │   ├── [x] struct AlertForwardingPlugin # activate 安装当前连接；try_publish 投递告警；disable_if_generation 只停用匹配代际
│   │       │   ├── [x] struct ConnectionGeneration # 用非零代际区分转发连接
│   │       │   ├── [x] struct ForwardingItem # 携带待交付告警与连接代际
│   │       │   ├── [x] struct AlertForwardingPluginStatus # 汇总配置、连接及投递状态
│   │       │   └── [x] enum AlertForwardingSubmitOutcome # 表达投递接收、过滤或丢弃结果
│   │       ├── [x] config.rs # 解析 JSON 转发配置并检查类别数量、格式和重复项
│   │       │   └── [x] struct AlertForwardingConfig # 持有启用开关、全部类别开关和指定类别列表
│   │       ├── [x] config_owner.rs # 将插件配置的读取和更新交给配置所有者
│   │       │   ├── [x] trait AlertForwardingConfigOwner # 提供持久配置读取与更新接口
│   │       │   └── [x] struct AlertForwardingConfigFileOwner # 从配置文件读取并保存插件配置
│   │       └── [x] filter.rs # 编译并匹配允许转发的告警类别
│   │           └── [x] struct CategoryFilter # 保存类别过滤条件
│   └── [x] sandbox_resource_alert/ # 消费 Guest 资源观测并产生独立 Sandbox 告警
│       └── [x] src/ # 阈值配置、插件注册与来源状态
│           ├── [x] lib.rs # 导出插件和配置，并重导出告警写入契约
│           ├── [x] plugin.rs # 注册观测兴趣、处理资源样本并提交告警
│           │   └── [x] struct SandboxResourceAlertPlugin # register 接入插件路由；publish_config 更新阈值配置
│           ├── [x] config.rs # 解析资源告警阈值与来源状态容量
│           │   └── [x] struct SandboxResourceAlertConfig # 保存 Guest 资源阈值和状态预算
│           └── [x] state.rs # 按观测来源保存计算告警所需的资源历史
│               ├── [x] struct SourceStateTable # 管理来源状态及有界容量
│               └── [x] struct SourceState # 保存单个来源的资源变化和告警计算状态
└── [x] tools/ # 探针发现、TLS 与存储诊断工具
```
