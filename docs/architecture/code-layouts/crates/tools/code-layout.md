状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/tools/ # TLS 探针识别、独立明文采集与存储诊断工具
├── [x] tls_probe_point_finder/ # 依据目标二进制、共享库和机器码生成 TLS 采集计划
│   └── [x] src/ # 二进制识别、检测树、候选选择与计划输出
│       ├── [x] lib.rs # 导出探针解析能力和错误类型
│       ├── [x] plan.rs # 定义目标身份、二进制和可安装探针
│       │   ├── [x] struct ProbePointPlan # 汇集目标的 TLS 探针位置和捕获策略
│       │   ├── [x] struct ProbePoint # 描述一个挂载点与载荷方向
│       │   └── [x] enum CaptureStrategy # 选择探针所使用的明文读取策略
│       ├── [x] resolve.rs # 解析目标文件及直接依赖的共享对象
│       │   └── [x] struct DirectSharedObjectResolver # 根据目标映射和解析模式定位共享库
│       ├── [x] binary_identity/ # 读取二进制身份所需的文件区域
│       │   └── [x] resolver.rs # 将探针输入绑定到实际二进制身份
│       │       └── [x] struct BinaryIdentityResolver # 解析二进制身份及其区域信息
│       ├── [x] elf/ # ELF 文件读取、符号查找和机器码扫描
│       │   ├── [x] source.rs # 可替换的探针文件读取入口
│       │   │   └── [x] trait ProbeFileSource # 提供检测所需文件读取能力
│       │   ├── [x] analysis_cache.rs # 复用同代文件的解析结果
│       │   │   └── [x] struct BinaryAnalysisCache # 按文件身份和代际保存二进制分析结果
│       │   └── [x] image.rs # 保存已解析 ELF 的架构、区段和内容
│       │       └── [x] struct ElfImage # 为符号和静态模式检测提供二进制视图
│       └── [x] probe_detector/ # 检测器契约与按 TLS 库组织的实现
│           ├── [x] contract/ # 检测上下文、候选、证据和选择策略
│           │   ├── [x] detector/ # 检测器行为接口
│           │   │   └── [x] probe_detector.rs # 单个检测节点的执行契约
│           │   │       └── [x] trait ProbeDetector # 使用检测上下文产生候选或拒绝结果
│           │   └── [x] selection/ # 根据证据与策略选择检测结果
│           │       └── [x] detection_selector.rs # 处理候选优先级和歧义
│           │           └── [x] struct DetectionSelector # 将候选集合转换为可解释的检测结果
│           └── [x] detector/ # TLS 检测器的组合实现
│               └── [x] tls/ # 按库、架构和发现方式分派检测
│                   ├── [x] probe_detector.rs # 组合各 TLS 库检测分支
│                   │   └── [x] struct TlsProbeDetector # 汇总可用检测器与选择结果
│                   ├── [x] openssl/ # OpenSSL 符号、可执行文件和共享库检测
│                   │   └── [x] probe_detector.rs # 选择 OpenSSL 架构与来源分支
│                   │       └── [x] struct OpenSslProbeDetector # 组织 OpenSSL 候选探针发现
│                   ├── [x] boringssl/ # BoringSSL 符号和架构专属静态模式检测
│                   │   └── [x] probe_detector.rs # 组合 BoringSSL 架构检测分支
│                   │       └── [x] struct BoringSslProbeDetector # 组织 BoringSSL 探针候选
│                   ├── [x] rustls/ # rustls 符号和静态模式检测
│                   │   └── [x] probe_detector.rs # 组合 rustls 检测方式
│                   │       └── [x] struct RustlsProbeDetector # 识别 rustls 连接和明文读取位置
│                   ├── [x] go_tls/ # Go TLS 元数据与函数检测
│                   │   └── [x] probe_detector.rs # 选择 Go TLS 的检测流程
│                   │       └── [x] struct GoTlsProbeDetector # 利用 Go 二进制信息定位 TLS 调用
│                   ├── [x] gnutls/ # GnuTLS 共享库检测
│                   │   └── [x] probe_detector.rs # 组合 GnuTLS 探针发现
│                   │       └── [x] struct GnuTlsProbeDetector # 定位 GnuTLS 明文函数
│                   └── [x] nss/ # NSS 共享库检测
│                       └── [x] probe_detector.rs # 组合 NSS 探针发现
│                           └── [x] struct NssProbeDetector # 定位 NSS 明文函数
├── [x] tls_payload_core/ # 同步明文处理的上下文、决策与等长改写
│   └── [x] src/ # 明文处理接口和规则实现
│       ├── [x] processor.rs # 定义同步处理输入和输出
│       │   ├── [x] struct PayloadContext # 携带方向、provider、符号和流标识
│       │   ├── [x] trait SyncProcessor # decide 根据上下文和明文字节产生决策
│       │   └── [x] enum Decision # 允许、阻断或等长替换当前明文
│       └── [x] rule.rs # 按规则执行等长内容替换
│           ├── [x] struct RewriteRule # 描述待匹配内容与替换内容
│           └── [x] struct EqualLenRewriteProcessor # 执行等长改写规则并返回处理决策
├── [x] tls_payload_probe/ # 独立启动目标并通过 BPF 观测 TLS 明文
│   └── [x] src/ # CLI、探针会话、流恢复和模型内容投影
│       ├── [x] lib.rs # 提供 run_from_env 与 main_from_env 工具入口
│       ├── [x] cli/ # 解析采集参数并输出字节、协议和模型报告
│       ├── [x] capture/ # 准备暂停目标、安装探针并消费载荷事件
│       │   ├── [x] session.rs # 组织启动、安装、恢复和退出后排空
│       │   │   └── [x] struct ProbeSession # run 驱动目标进程与采集循环并返回退出状态
│       │   ├── [x] ebpf.rs # 创建采集 BPF 对象并消费事件缓冲
│       │   │   └── [x] struct BpfPayloadRuntime # load 安装目标探针；poll_events、finish_events 交付采集结果
│       │   └── [x] assembly/ # 恢复 HTTP、SSE 与 WebSocket 字节流
│       │       └── [x] http_stream.rs # 保存增量 HTTP 消息恢复状态
│       │           └── [x] struct StreamingState # 累积未完整接收的协议内容
│       └── [x] llm_projection/ # 将捕获的模型请求响应转换为可读语义
│           └── [x] router.rs # 选择 provider 解析并维护请求流关联
│               └── [x] struct LlmProjector # 组织模型请求、响应与流式投影
├── [x] tls_payload_probe_sync/ # 注入目标进程的同步 TLS 采集和明文处理
│   └── [x] src/ # 工具入口、注入配置和 hook 运行时
│       ├── [x] lib.rs # 提供同步探针 CLI 入口
│       ├── [x] cli/ # 解析目标与规则，启动注入并输出报告
│       └── [x] runtime/ # 管理注入进程内的探针、连接和处理状态
│           ├── [x] config/ # 注入配置解析和运行计划组装
│           │   ├── [x] factory.rs # 从环境和配置创建注入依赖
│           │   │   └── [x] struct RuntimeConfigFactory # 组装 RuntimeBootstrap 与实际运行配置
│           │   ├── [x] state.rs # 保存 hook 点、过滤和事件传输配置
│           │   │   └── [x] struct RuntimeConfig # 持有注入运行时生效配置
│           │   └── [x] plan.rs # 保存待安装的运行计划
│           │       └── [x] struct RuntimePlan # 携带同步探针安装信息
│           ├── [x] hook/ # 实现 x86_64 与 aarch64 指令重定位和函数跳转
│           ├── [x] tls/ # TLS 函数的动态定位、绑定和调用包装
│           │   └── [x] dynamic/ # 管理运行中发现的函数目标
│           │       ├── [x] binding/ # 解析目标位置并记录绑定审计
│           │       └── [x] core/ # 保存目标槽位并执行捕获包装
│           │           └── [x] slot_table.rs # 管理动态 TLS 目标槽位
│           │               └── [x] struct SlotTable # 持有已分配目标及其槽位索引
│           └── [x] flow_control/ # 按流恢复协议并执行有界明文处理
│               └── [x] controller.rs # 协调流状态、淘汰与输出动作
│                   └── [x] struct FlowController # 管理每条流的协议恢复与内容处理
├── [ ] graph_inspect/ # 图导出诊断工具的包骨架；查询和报告待实现
│   └── [x] src/ # 已建立的命令输入、入口与报告模块位置
│       ├── [x] lib.rs # 声明 args、entry 和 report 模块
│       ├── [ ] args.rs # 命令输入形状占位，待实现参数解析
│       ├── [ ] entry.rs # 图检查入口占位，待实现执行流程
│       └── [ ] report.rs # 图检查输出占位，待实现结果报告
└── [ ] sqlite_inspect/ # SQLite trace 诊断工具的包骨架；查询和报告待实现
    └── [x] src/ # 已建立的命令输入、入口与报告模块位置
        ├── [x] lib.rs # 声明 args、entry 和 report 模块
        ├── [ ] args.rs # 命令输入形状占位，待实现参数解析
        ├── [ ] entry.rs # SQLite 检查入口占位，待实现执行流程
        └── [ ] report.rs # 存储检查输出占位，待实现结果报告
```
