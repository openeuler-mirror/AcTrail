状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/adapters/ # 将内核采集、控制传输、隔离链路和外部数据表示接入领域契约
├── [x] collectors/ # 将 Linux 内核及 procfs 信号转换为观测
│   ├── [x] ebpf/ # 挂载 BPF 程序、维护进程采集绑定并解码事件缓冲
│   │   ├── [x] bpf/ # 内核事件 ABI、共享运行时与各领域采集程序
│   │   └── [x] src/ # 用户态 BPF 加载、绑定、事件解码和采集轮询
│   │       ├── [x] lib.rs # 聚合采集配置、绑定状态与运行实例
│   │       │   └── [x] struct EbpfCollector # 实现 CollectorInstance，管理绑定并返回原始观测及明文段
│   │       ├── [x] loader.rs # 定义 BPF 加载器及已挂载对象的资源所有权
│   │       │   ├── [x] struct EbpfProgramLoader # 持有内核、明文及文件采集配置，准备观测与可选 LSM 装配
│   │       │   └── [x] struct EbpfRuntime # 拥有事件消费者、Object、Link、绑定 map 和 TLS 状态；先停消费者再释放 BPF 对象
│   │       ├── [x] maps.rs # 维护 trace 对目标进程的采集绑定
│   │       │   ├── [x] struct BindingStateMap # 保存当前绑定的用户态状态
│   │       │   └── [x] struct TrackedProcess # 记录跟踪进程和所需采集范围
│   │       ├── [x] procfs.rs # 从 procfs 读取身份和进程树
│   │       │   ├── [x] struct ProcfsIdentityReader # 读取并核验进程身份坐标
│   │       │   └── [x] struct ProcfsTreeSnapshotter # 实现进程树快照查询
│   │       ├── [x] capability_probe.rs # 探测内核、tracefs 和 BPF 能力
│   │       │   └── [x] struct EbpfProbeResult # 返回环境探测结果与能力可用性
│   │       ├── [x] loader/ # 装配挂载计划与运行中事件处理
│   │       │   ├── [x] attach_plan/ # 将配置能力转换为实际挂载集合
│   │       │   │   └── [x] implementation.rs # 保存最终 BPF 程序挂载计划
│   │       │   │       └── [x] struct AttachPlan # 表达已选择的采集能力与程序
│   │       │   ├── [x] ring_decode.rs # 解码公共事件头并分发领域载荷
│   │       │   │   ├── [x] enum KernelEvent # 区分观测、载荷和采集诊断事件
│   │       │   │   └── [x] struct KernelEventIdentity # 保留内核事件携带的进程身份
│   │       │   └── [x] tls/ # TLS 动态探针、待完成采集及诊断
│   │       │       ├── [x] dynamic.rs # 保存运行中解析到的 TLS 探针计划
│   │       │       │   └── [x] struct DynamicTlsProbePlan # 携带可安装的动态 TLS 采集计划
│   │       │       └── [x] pending.rs # 管理尚未完成的 TLS 明文操作
│   │       │           └── [x] struct PendingTlsPayloadOp # 关联调用上下文与完成采集结果
│   │       └── [x] collector/ # 原始采集事件的领域处理
│   │           └── [x] net_aggregation.rs # 聚合网络观测并按阈值刷新
│   │               ├── [x] struct NetAggregator # 持有网络聚合状态
│   │               └── [x] trait FlushPolicy # 决定网络聚合批次刷新时机
│   ├── [x] libbpf_tracepoint/ # 共享 tracepoint 挂载和必需能力检查
│   │   └── [x] src/ # tracepoint 挂载适配
│   │       └── [x] attacher.rs # 区分必需与可选挂载点并返回结果
│   │           ├── [x] struct TracepointProgramAttacher # 为 libbpf 程序建立 tracepoint 挂载
│   │           └── [x] enum TracepointRequirement # 表达挂载点的必需程度
│   └── [x] sandbox_linux/ # Guest 进程 I/O、整体资源、压力及 cgroup 采集
│       └── [x] src/ # Guest Linux 观测来源及采样周期
│           ├── [x] collector.rs # 组织进程 I/O 和资源采样
│           │   ├── [x] struct SandboxLinuxCollector # 组合 Linux 采样来源并返回观测周期
│           │   └── [x] struct SandboxProcessIoCollector # 采集 Guest 进程 I/O 与内核诊断
│           ├── [x] resource.rs # 从 Linux 接口读取整体资源
│           │   └── [x] struct LinuxResourceReader # 读取 CPU 与内存资源快照
│           ├── [x] psi.rs # 读取 Linux 压力停顿信息
│           │   └── [x] struct PsiReader # 解析 Guest CPU、内存和 I/O 压力
│           ├── [x] workload.rs # 发现并采样 Guest 内工作负载 cgroup
│           │   └── [x] struct WorkloadCgroupCollector # 读取工作负载计数和来源身份
│           └── [x] config.rs # Guest 采集开关、周期和容量配置
│               └── [x] struct SandboxLinuxConfig # 持有 Guest Linux 采集设置
├── [x] control/ # daemon 控制命令的进程间通信
│   └── [x] uds/ # 使用 Unix socket 传输控制命令及附带文件描述符
│       ├── [x] client/ # ctl 使用的控制请求客户端
│       │   └── [x] src/ # 请求往返与 socket 连接实现
│       │       └── [x] lib.rs # 定义传输接口并组合控制客户端
│       │           ├── [x] trait RoundTripTransport # 执行一次请求与响应往返
│       │           ├── [x] struct UdsSocketTransport # 实现 Unix socket 传输
│       │           └── [x] struct UdsControlClient # 将控制命令交给传输并返回领域响应
│       ├── [x] server/ # daemon 使用的 Unix socket 控制服务
│       │   └── [x] src/ # 连接接收、对端身份与领域分发
│       │       └── [x] lib.rs # 接收连接并调用注入的控制服务
│       │           ├── [x] trait ControlService # 处理已解码控制请求
│       │           ├── [x] struct UdsControlServer # 持有控制监听入口和服务实现
│       │           └── [x] struct PeerCredentials # 保留对端进程凭据
│       └── [x] transport/ # 控制帧及文件描述符传输编解码
│           └── [x] src/ # Unix socket 协议实现
│               └── [x] lib.rs # 编码控制请求响应并处理描述符传递
├── [x] agent_host/ # 将 Agent launch 与执行状态接入观测控制
│   └── [x] src/ # Agent 宿主配置和执行生命周期
│       ├── [x] launch.rs # 安装 Agent 启动时所需的集成信息
│       │   └── [x] struct AgentLaunchIntegration # 准备 Agent launch 接入
│       ├── [x] execution.rs # 跟踪 Agent 执行标识与状态
│       │   └── [x] struct ExecutionStates # 管理 ExecutionKey 对应的执行状态
│       └── [x] config.rs # Agent 宿主接入配置
│           └── [x] struct AgentHostConfig # 保存 Agent 集成设置
├── [x] provider/ # 根据证据识别模型服务提供方
│   └── [x] rule_set/ # 内置 provider 规则集适配
│       └── [x] src/ # 规则配置、规则条目和匹配器
│           ├── [x] classifier.rs # 实现 provider 证据分类
│           │   └── [x] struct RuleSetClassifier # 依据 ProviderRule 匹配证据并产生标签
│           └── [x] rules.rs # 定义单条 provider 判定规则
│               └── [x] struct ProviderRule # 保存提供方判定所需的匹配条件
├── [x] export/ # 将已有记录转换为对外导出表示
│   ├── [x] json_graph/ # 查询记录并生成 JSON 图文档
│   │   └── [x] src/ # 图导出服务
│   │       └── [x] service.rs # 从只读来源构造 GraphDocument
│   │           └── [x] struct JsonGraphExportService # 借用数据来源并导出图节点和关系
│   └── [x] otel/ # 为调用方提供 OTEL 编码能力入口
│       └── [x] src/ # OTEL 导出适配公开接口
│           └── [x] lib.rs # 重导出 OTEL codec 能力
├── [x] sandbox_control/ # Guest 本地管理传输
│   └── [x] uds/ # Guest 控制命令的 Unix socket 客户端、服务端与编解码
│       └── [x] src/ # 控制连接、请求处理与运行句柄
│           ├── [x] client.rs # 向 Guest daemon 发起本地控制请求
│           │   └── [x] struct SandboxControlUdsClient # 执行 Guest 管理请求往返
│           ├── [x] server.rs # 接收 Guest 管理连接并调度请求
│           │   ├── [x] struct SandboxControlUdsServer # 持有服务配置与本地控制端口
│           │   └── [x] struct SandboxControlServerHandle # 管理已启动控制服务的生命周期
│           └── [x] codec.rs # 对 Guest 控制命令和响应进行有界编解码
│               └── [x] struct SandboxControlCodec # 实现本地控制协议转换
├── [x] sandbox_link/ # Guest 与 gateway 连接及 daemon 上游监听
│   ├── [x] vsock/ # 内核 VSOCK 与 Unix stream 连接适配
│   │   └── [x] src/ # 客户端、监听器和连接操作
│   │       ├── [x] client.rs # 按配置建立 Guest 侧链路
│   │       │   └── [x] struct VsockTransportFactory # 根据 VsockTransportConfig 创建传输
│   │       ├── [x] connection.rs # 统一两类连接的读写行为
│   │       │   └── [x] enum VsockConnection # 持有内核 VSOCK 或 Unix stream 连接
│   │       └── [x] listener.rs # 接收隔离链路连接
│   │           └── [x] enum VsockListener # 持有对应传输的监听端点
│   └── [x] upstream/ # daemon 侧 gateway TCP 接入
│       └── [x] src/ # 上游监听、连接与服务状态
│           ├── [x] server.rs # 接收网关连接并执行关闭流程
│           │   └── [x] struct UpstreamTcpServer # 持有 TCP 服务和连接生命周期
│           └── [x] config.rs # 上游端点与连接预算
│               └── [x] struct UpstreamServerConfig # 保存监听地址与服务容量
└── [x] storage/ # 隔离观测证据和插件告警的独立 SQLite 适配
    ├── [x] sandbox/ # 保存无插件兴趣的 Guest 观测
    │   └── [x] src/ # 异步写入、只读查询与存储配置
    │       ├── [x] lib.rs # 组合证据存储的写入和生命周期能力
    │       │   └── [x] struct SandboxEvidenceSqliteStore # 提供证据接收、状态与关闭端口
    │       ├── [x] reader.rs # 读取已存证据
    │       │   └── [x] struct SandboxEvidenceSqliteReader # 实现独立证据库的只读查询
    │       └── [x] config.rs # SQLite 路径、队列和同步设置
    │           └── [x] struct SandboxEvidenceSqliteConfig # 保存证据数据库配置
    └── [x] sandbox_alert/ # 保存插件产生的隔离资源等告警
        └── [x] src/ # 告警写入、读取与关闭能力
            ├── [x] lib.rs # 组合告警存储端口
            │   └── [x] struct SandboxAlertSqliteStore # 提供告警接收、提交确认与生命周期管理
            ├── [x] reader.rs # 查询独立告警数据库
            │   └── [x] struct SandboxAlertSqliteReader # 实现隔离告警读取端口
            └── [x] config.rs # 告警数据库与队列配置
                └── [x] struct SandboxAlertSqliteConfig # 保存告警 SQLite 配置
```
