状态标记：`[ ]` 待编写，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
./ # 项目文档布局
├── [x] README.md # 项目介绍（英文）
├── [x] README.zh-CN.md # 项目介绍（中文）
├── [x] docs/ # 产品与开发文档
│   ├── [x] README.md # 文档导航
│   ├── [x] architecture/ # 架构文档
│   │   ├── [x] README.md # 架构阅读入口
│   │   ├── [/] c4-graphs.md # 架构图索引
│   │   ├── [x] sequence-graphs.md # 跨组件交互时序图
│   │   ├── [x] code-layout.md # 代码职责布局
│   │   ├── [x] code-layout.html # 代码布局浏览页
│   │   ├── [x] code-layouts/ # 模块代码布局
│   │   ├── [x] config-layout.md # 配置字段表
│   │   ├── [x] doc-layout.md # 文档职责布局
│   │   ├── [x] system/ # 系统关系图示
│   │   │   ├── [x] assets/ # 系统图资源
│   │   │   ├── [x] containers.md # 运行角色关系图
│   │   │   ├── [x] context.md # 系统边界关系图
│   │   │   ├── [x] key-data-flows.md # 系统数据流图
│   │   │   └── [x] product-landscape.md # 产品组合关系图
│   │   ├── [x] deployment/ # 部署关系图示
│   │   │   ├── [x] assets/ # 部署图资源
│   │   │   ├── [x] default.md # 默认部署图
│   │   │   └── [x] execution-isolation.md # 执行隔离部署图
│   │   └── [x] components/ # 组件设计
│   │       ├── [x] alert-proxy.md # 告警代理设计
│   │       ├── [x] assets/ # 组件图资源
│   │       ├── [x] core-runtime.md # 核心运行时设计
│   │       ├── [x] ebpf-event-abi.md # 内核事件接口设计
│   │       ├── [x] ebpf-event-transport.md # 内核事件传输设计
│   │       ├── [x] execution-isolation.md # 隔离运行时设计
│   │       ├── [x] live-tool-projector.md # 工具调用投影设计
│   │       ├── [x] llm-request-projection.md # 模型请求投影设计
│   │       ├── [x] llm-trajectory-graph.md # 模型轨迹视图设计
│   │       ├── [x] mcp-stdio-observation.md # MCP 标准流观测设计
│   │       ├── [x] plugin-runtime.md # 插件运行时设计
│   │       ├── [x] probe-detector.md # 探针检测器设计
│   │       ├── [x] process-identity-runtime.md # 进程身份设计
│   │       ├── [x] resource-profiling/ # 资源观测组件文档
│   │       │   ├── [x] configuration.md # 资源配置使用说明
│   │       │   ├── [x] design.md # 资源观测设计
│   │       │   └── [x] storage.md # 资源样本存储设计
│   │       ├── [x] streaming-parser.md # 流式解析设计
│   │       ├── [x] tls-sync-runtime.md # TLS 同步采集设计
│   │       ├── [x] web-frontend/ # 前端设计
│   │       │   ├── [x] README.md # 前端设计入口
│   │       │   ├── [x] assets/ # 前端图资源
│   │       │   ├── [x] controls-and-layout.md # 控件与布局设计
│   │       │   ├── [x] controls/ # 控件交互设计
│   │       │   │   └── [x] navigation-workspace.md # 导航与工作区设计
│   │       │   └── [x] runtime-and-delivery.md # 前端运行与交付设计
│   │       └── [x] web-runtime.md # Web 服务设计
│   ├── [x] concepts/ # 产品概念
│   │   ├── [x] capabilities.md # 产品能力概览
│   │   ├── [x] collection-observation-checklist.md # 采集与观测清单
│   │   ├── [x] runtime-model.md # 运行模型说明
│   │   └── [x] security-model.md # 安全边界说明
│   ├── [x] decisions/ # 架构决策与校验许可
│   │   ├── [x] README.md # 决策与许可导航
│   │   ├── [x] 0001-isolate-hand-observation-path.md # 手侧观测隔离决策依据
│   │   ├── [x] 0002-use-pidfd-for-launch-registration.md # 进程身份注册决策依据
│   │   ├── [x] 0003-use-vsock-bridge-for-networkless-guest-egress.md # 无网络出境决策依据
│   │   ├── [x] 0004-use-a-standalone-alert-proxy.md # 独立告警代理决策依据
│   │   └── [x] verifications/ # 校验许可记录
│   │       └── [x] denied_and_approved.md # 校验内容与许可状态
│   ├── [x] getting-started/ # 入门指南
│   │   └── [x] quickstart.md # 首次运行指南
│   ├── [x] operations/ # 运维操作指南
│   │   ├── [x] README.md # 操作导航
│   │   ├── [x] daemon/ # 守护进程操作
│   │   │   ├── [x] configure.md # 配置操作
│   │   │   ├── [x] inspect-traces.md # 观测记录查询与导出
│   │   │   └── [x] start-stop.md # 启停与状态检查
│   │   ├── [x] deployment/ # 部署指南
│   │   │   ├── [x] choose-a-mode.md # 部署模式选择
│   │   │   ├── [x] container.md # 容器观测部署
│   │   │   ├── [x] execution-isolation.md # 执行隔离部署
│   │   │   └── [x] host.md # 主机部署
│   │   ├── [x] idle-detection.md # 执行停滞检测操作
│   │   ├── [x] integrations/ # Agent 接入指南
│   │   │   └── [x] xiaoo-rustls.md # xiaoO 观测接入
│   │   ├── [x] plugins/ # 插件操作指南
│   │   │   ├── [x] README.md # 操作导航
│   │   │   ├── [x] activity-anomaly.md # 异常活动插件启用
│   │   │   ├── [x] manage.md # 插件管理操作
│   │   │   ├── [x] otel-http.md # HTTP 导出插件启用
│   │   │   └── [x] otel-jsonl.md # JSONL 导出插件启用
│   │   ├── [x] resource-metrics-containers.md # 容器资源计量操作
│   │   ├── [x] resource-metrics-host.md # 主机与任务资源观测操作
│   │   ├── [x] resource-metrics-sandboxes.md # 隔离环境资源计量操作
│   │   └── [x] troubleshooting/ # 故障排查指南
│   │       ├── [x] capture.md # 采集缺失排查
│   │       ├── [x] deployment.md # 部署启动故障排查
│   │       ├── [x] plugins.md # 插件故障排查
│   │       └── [x] preflight.md # 部署前检查
│   ├── [x] reference/ # 配置与接口参考
│   │   ├── [x] README.md # 参考资料导航
│   │   ├── [x] configuration/ # 配置参考
│   │   │   ├── [x] collection.md # 采集与保留配置参考
│   │   │   └── [x] daemon.md # 守护进程配置参考
│   │   ├── [x] data-model/ # 数据模型参考
│   │   │   ├── [x] export.md # 导出动作模型参考
│   │   │   ├── [x] llm-request.md # 模型请求数据参考
│   │   │   ├── [x] sandbox-alert.md # 沙箱告警数据参考
│   │   │   └── [x] sandbox-observation.md # 沙箱观测数据参考
│   │   ├── [x] platform-support.md # 平台支持范围
│   │   ├── [x] plugin-api/ # 插件接口参考
│   │   │   ├── [x] README.md # 接口导航
│   │   │   ├── [x] control-decider.md # 控制决策接口参考
│   │   │   ├── [x] llm-codec.md # 模型编解码接口参考
│   │   │   ├── [x] observation-consumer.md # 观测消费接口参考
│   │   │   └── [x] wasm-core-module.md # WASM 模块接口参考
│   │   └── [x] protocols/ # 通信协议参考
│   │       └── [x] alert-proxy.md # 告警代理协议参考
│   └── [x] specifications/ # 实现行为要求
│       ├── [x] README.md # 规范导航
│       ├── [x] export/ # 动作与交付要求
│       │   ├── [x] action-delivery.md # 语义动作交付要求
│       │   └── [x] file-io-terminal-actions.md # 文件读写终态要求
│       ├── [x] identity/ # 进程身份要求
│       │   └── [x] launch-trace-registration.md # 启动观测注册要求
│       ├── [x] isolation/ # 执行隔离要求
│       │   ├── [x] alert-delivery.md # 告警交付隔离要求
│       │   └── [x] runtime-lifecycle.md # 隔离运行生命周期要求
│       ├── [x] observation/ # 采集与解析要求
│       │   ├── [x] mcp-stdio.md # MCP 标准流观测要求
│       │   ├── [x] probe-detector.md # 探针发现要求
│       │   ├── [x] sandbox-observation.md # 沙箱观测要求
│       │   └── [x] streaming-parser.md # 流式解析要求
│       └── [x] plugins/ # 插件运行要求
│           ├── [x] README.md # 规范导航
│           ├── [x] lifecycle-and-isolation.md # 插件生命周期与隔离要求
│           └── [x] otel-exporters.md # OTEL 导出要求
├── [x] deploy/ # 部署资产说明
│   ├── [x] agent-host/ # Agent 接入文档
│   │   └── [x] README.md # 接入指南
│   ├── [x] alert-proxy/ # 告警代理部署文档
│   │   └── [x] README.md # 部署指南
│   ├── [x] container-auto/ # 容器部署文档
│   │   ├── [x] README.md # 部署指南
│   │   └── [x] seccomp/ # 系统调用策略文档
│   │       └── [x] README.md # 策略使用说明
│   ├── [x] execution-isolation/ # 执行隔离部署文档
│   │   └── [x] README.md # 部署指南
│   └── [x] virtual-container/ # 虚拟容器部署文档
│       ├── [x] README.md # 部署指南
│       ├── [x] guest/ # Guest 环境文档
│       │   └── [x] OPENEULER.md # openEuler 部署指南
│       ├── [x] host-collector/ # Collector 部署文档
│       │   └── [x] README.md # 部署指南
│       ├── [x] vsock-egress/ # VSOCK 出境文档
│       │   └── [x] README.md # 部署指南
│       └── [x] workload/ # 工作负载接入文档
│           └── [x] README.md # 接入指南
├── [x] crates/ # 随源码维护的文档
│   ├── [x] access_control/ # 访问控制组件文档
│   │   └── [x] README.md # 组件设计
│   ├── [x] apps/ # 应用开发文档
│   │   └── [x] web/ # Web 应用文档
│   │       └── [x] frontend/ # 前端开发文档
│   │           └── [x] README.md # 开发指南
│   └── [x] tools/ # 开发工具文档
│       ├── [x] tls_payload_probe/ # TLS 探针工具文档
│       │   └── [x] README.md # 工具使用说明
│       ├── [x] tls_payload_probe_sync/ # TLS 同步探针文档
│       │   └── [x] README.md # 工具使用说明
│       └── [x] tls_probe_point_finder/ # TLS 探针定位器文档
│           ├── [x] README.md # 工具使用说明
│           └── [x] src/ # 内部设计文档
│               └── [x] probe_detector/ # 探针检测设计
│                   └── [x] detector/ # 检测器设计
│                       └── [x] tls/ # TLS 检测设计
│                           └── [x] rustls/ # rustls 检测设计
│                               └── [x] handshake_locator/ # 握手定位设计
│                                   └── [x] README.md # 定位算法说明
├── [x] examples/ # 功能示例文档
│   └── [x] plugins/ # 插件示例文档
│       ├── [x] README.zh.md # 示例导航
│       ├── [x] builtin/ # 内置插件示例
│       │   ├── [x] alert-forwarding/ # 告警转发
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] otel-http/ # OTLP HTTP 导出
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] otel-jsonl/ # OTEL JSON 导出
│       │   │   └── [x] README.zh.md # 示例说明
│       │   └── [x] sandbox-resource-alert/ # 隔离环境资源告警
│       │       └── [x] README.zh.md # 示例说明
│       ├── [x] wasm-legacy/ # Wasm 插件示例
│       │   ├── [x] control-command-deny/ # 命令拒绝决策
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] control-graylist/ # 文件灰名单决策
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] control-network-deny/ # 网络拒绝决策
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] llm-codec-noop/ # 编解码接口示例
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] llm-codec-qoder/ # Qoder 编解码器
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] observation-count/ # 观测记录计数
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] observation-env-read/ # 环境变量读取
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] observation-payload-read/ # 观测内容读取
│       │   │   └── [x] README.zh.md # 示例说明
│       │   ├── [x] tool-consecutive-failure-alert/ # 工具连续失败告警
│       │   │   └── [x] README.zh.md # 示例说明
│       │   └── [x] tool-frequent-failure-alert/ # 工具频繁失败告警
│       │       └── [x] README.zh.md # 示例说明
│       └── [x] wit-component/ # 组件插件示例
│           ├── [x] activity-anomaly/ # 活动异常告警
│           │   └── [x] README.zh.md # 示例说明
│           ├── [x] command-policy-dynamic/ # 动态命令策略
│           │   ├── [x] README.zh.md # 示例说明
│           │   └── [x] command-security-plugins-experience.zh.md # 功能体验指南
│           ├── [x] control-graylist/ # 文件灰名单决策
│           │   └── [x] README.zh.md # 示例说明
│           ├── [x] control-hostcalls/ # 控制宿主接口
│           │   └── [x] README.zh.md # 示例说明
│           ├── [x] file-leakage/ # 文件泄露告警
│           │   ├── [x] README.md # 英文使用说明
│           │   └── [x] README.zh.md # 中文使用说明
│           ├── [x] file-policy-dynamic/ # 动态文件策略
│           │   ├── [x] README.zh.md # 示例说明
│           │   └── [x] file-security-plugins-experience.zh.md # 功能体验指南
│           ├── [x] llm-turn-anomaly/ # 模型会话异常告警
│           │   └── [x] README.md # 示例说明
│           ├── [x] network-policy-dynamic/ # 动态网络策略
│           │   └── [x] README.zh.md # 示例说明
│           ├── [x] observation-payload-read/ # 观测内容读取
│           │   └── [x] README.zh.md # 示例说明
│           └── [x] observation-read-config/ # 插件配置读取
│               └── [x] README.zh.md # 示例说明
├── [x] tests/ # 测试文档
│   ├── [x] README.md # 测试导航
│   ├── [x] plugins/ # 插件测试文档
│   │   ├── [x] dynamic-builtin/ # 内置插件动态加载
│   │   │   └── [x] README.md # 测试指南
│   │   ├── [x] lifecycle/ # 插件生命周期
│   │   │   └── [x] README.md # 测试指南
│   │   ├── [x] otel-jsonl/ # OTEL JSON 导出
│   │   │   └── [x] README.md # 测试指南
│   │   ├── [x] persistent-load/ # 插件持久加载
│   │   │   └── [x] README.md # 测试指南
│   │   ├── [x] wasm-component-control-graylist/ # 组件灰名单决策
│   │   │   └── [x] README.md # 测试指南
│   │   ├── [x] wasm-component-control-hostcalls/ # 组件宿主接口
│   │   │   └── [x] README.md # 测试指南
│   │   └── [x] wasm-observation/ # Wasm 观测插件
│   │       └── [x] README.md # 测试指南
│   └── [x] v2/ # 回归测试文档
│       ├── [x] README.md # 测试约定
│       ├── [x] common/ # 公共测试文档
│       │   ├── [x] llm-http-proxy/ # 模型代理服务
│       │   │   └── [x] README.md # 使用说明
│       │   └── [x] test_suites/ # 公共套件文档
│       │       └── [x] local_maas_server/ # 本地模型服务
│       │           ├── [x] README.md # 使用说明
│       │           └── [x] docs/ # 专题说明
│       │               ├── [x] architecture.md # 架构说明
│       │               ├── [x] https.md # HTTPS 使用说明
│       │               ├── [x] protocol-adapters.md # 协议适配说明
│       │               ├── [x] record-mode.md # 录制模式说明
│       │               ├── [x] response-scheduling.md # 响应调度说明
│       │               └── [x] scenario-format.md # 剧本格式参考
│       └── [x] regression/ # 回归测例文档
│           ├── [x] README.md # 回归指南
│           ├── [x] activity_anomaly/ # 活动异常告警
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] alert_forwarding/ # 告警转发
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] command_policy_xiaoo/ # 命令治理
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] container_agent_xiaoo/ # 容器 Agent 观测
│           │   ├── [x] README.zh.md # 测试指南
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] container_auto/ # 容器权限选择
│           │   ├── [x] README.zh.md # 测试指南
│           │   ├── [x] v1/ # v1 测例说明
│           │   │   └── [x] README.zh.md # 测试指南
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] execution_isolation_cloud_hypervisor/ # Cloud Hypervisor 隔离
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] execution_isolation_firecracker/ # Firecracker 隔离
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] execution_isolation_stratovirt/ # StratoVirt 隔离
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] file_policy_xiaoo/ # 文件治理
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] http_llm_sources/ # 协议消息来源
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] lsm_access_control_xiaoo/ # 访问控制后端
│           │   ├── [x] README.zh.md # 测试指南
│           │   └── [x] performance/ # 性能测量
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] network_policy_xiaoo/ # 网络治理
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] otel_http/ # OTEL HTTP 导出
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] otel_jsonl_action_filter/ # 导出动作筛选
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_claude_llm/ # Claude 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_claude_mcp/ # Claude MCP 观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_codex_llm/ # Codex 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_codex_mcp/ # Codex MCP 观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_dcode_llm/ # dcode 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_pi_llm/ # pi 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_qodercli_llm/ # Qoder 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] probe_xiaoo_llm/ # xiaoO 模型观测
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] project_subagent_trajectory/ # 子代理轨迹
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] resource_metrics_cgroup/ # 工作负载资源计量
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] sandbox_oom_killed_alert_host/ # 隔离进程 OOM 告警
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] sandbox_resource_alert_host/ # 隔离环境资源告警
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] semantic_action_boundaries/ # 语义动作边界
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] tool_consecutive_failure_alert/ # 工具连续失败告警
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] tool_frequent_failure_alert/ # 工具频繁失败告警
│           │   └── [x] README.zh.md # 测试指南
│           ├── [x] virtual_container/ # 虚拟容器观测
│           │   ├── [x] README.zh.md # 测试指南
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           ├── [x] virtual_container_xiaoo_concurrency/ # 虚拟容器并发观测
│           │   ├── [x] README.zh.md # 测试指南
│           │   └── [x] v2/ # v2 测例说明
│           │       └── [x] README.zh.md # 测试指南
│           └── [x] web_timeline/ # 时间线交互
│               └── [x] README.md # 测试指南
├── [x] scripts/ # 脚本使用文档
│   └── [x] bench/ # 性能与验收文档
│       ├── [x] detail/ # 操作开销测量
│       │   └── [x] README.md # 运行说明
│       ├── [x] overall/ # Agent 整体开销
│       │   └── [x] README.md # 运行说明
│       └── [x] payload/ # 采集性能与验收
│           ├── [x] README.md # 运行说明
│           ├── [x] ablation/ # CPU 开销归因
│           │   └── [x] README.md # 运行说明
│           ├── [x] action_state/ # 动作生命周期
│           │   └── [x] README.md # 运行说明
│           ├── [x] file_state/ # 文件状态观测
│           │   └── [x] README.md # 运行说明
│           ├── [x] storage_cpu/ # 存储 CPU 开销
│           │   └── [x] README.md # 运行说明
│           ├── [x] storage_delivery/ # 存储交付故障
│           │   └── [x] README.md # 运行说明
│           ├── [x] storage_independence/ # 存储独立性
│           │   └── [x] README.md # 运行说明
│           ├── [x] tls_library/ # TLS 共享库发现
│           │   └── [x] README.md # 运行说明
│           ├── [x] tls_library_scaling/ # 共享库规模测量
│           │   └── [x] README.md # 运行说明
│           ├── [x] tls_mapping/ # TLS 映射边界
│           │   └── [x] README.md # 运行说明
│           ├── [x] tls_scaling/ # TLS 采集规模测量
│           │   └── [x] README.md # 运行说明
│           └── [x] tls_write/ # TLS 写入完成
│               └── [x] README.md # 运行说明
└── [x] images/ # 项目介绍插图
```
