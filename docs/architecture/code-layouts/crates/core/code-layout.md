状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/ # 领域状态与运行协调；应用组合具体采集器、存储和传输端口。
├── [x] config/ # 组合 operator 配置，生成默认文档并校验采集能力及模块配置。
├── [x] model/ # 定义事件、trace、membership、能力、诊断和载荷的共享领域值。
├── [x] process_identity/ # 拥有稳定进程身份及带代际的坐标索引，识别 PID 复用和身份冲突。
├── [x] trace_runtime/ # 拥有活跃 trace、membership、所有者及采集能力协商结果。
├── [x] attach_runtime/ # 协调已有进程的身份读取、树快照、采集绑定和覆盖窗口。
├── [x] ingest_runtime/ # 将精确匹配的采集事件规范化，附加观测策略和 provider 标签。
├── [x] semantic_action_runtime/ # 将事件及明文协议投影为语义动作、关系、内容和 lineage。
├── [x] plugin_system/ # 定义插件能力、授权、宿主端口与 Sandbox 消费者注册表。
├── [x] plugin_wasm_runtime/ # 承载 WASM 实例、受限宿主调用与 observation/control/codec 适配。
├── [x] resource_profiling/ # 拥有 Host/Workload 资源采集、指标、存储、告警及配置。
├── [x] idle_detector/ # 依据 agent 执行状态识别挂起区间，管理检测与恢复告警。
├── [x] tls_payload_sync/ # 定义 TLS-sync 计划、注入环境、帧协议及事件发送客户端。
├── [x] sandbox_agent_runtime/ # 协调 Guest 采样线程、工作单元会话和有界观测投递。
├── [x] vsock_gateway_runtime/ # 管理 Guest 连接身份、VSOCK 会话配额与 TCP 上游发送。
├── [x] gateway_ingest_runtime/ # 在 daemon 侧限制网关连接并向隔离观测 sink 交付批次。
├── [x] linux_cgroup/ # 以目录身份约束 cgroup v2 读取、受管范围创建和空目录清理。
└── [x] linux_platform/ # 提供 Linux 能力探测、seccomp syscall 映射及 cgroup 适配入口。
```
