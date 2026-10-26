# 架构图

架构图按系统上下文、运行容器、组件和部署位置组织。各专题页展示图与说明。交互顺序见[运行时序](sequence-graphs.md)，源码归属见[代码布局](code-layouts/crates/code-layout.md)。

## 系统上下文与运行容器

| 视图 | 阅读入口 |
| --- | --- |
| 系统上下文 | [系统边界与使用者](system/context.md) |
| 产品组合 | [本地节点与集群中心](system/product-landscape.md) |
| 本地运行容器 | [daemon、ctl、Web、Viewer 与存储](system/containers.md#本地节点) |
| 集群运行容器 | [reporter、center 与 cluster root](system/containers.md#集群中心) |
| 执行隔离运行容器 | [Guest、gateway 与隔离存储](system/containers.md#执行隔离路径) |

运行容器表示承担独立职责的进程或存储。TLS 明文默认采用 eBPF `bpf-copy` 采集；本地运行图中的 TLS-sync runtime 表示选择该 backend 时加载到目标进程的组件。

## 组件视图

| 组件 | 阅读入口 |
| --- | --- |
| daemon 核心运行时 | [服务组合与故障边界](components/core-runtime.md) |
| 进程身份 | [PID 坐标与 fork/exec 时序](components/process-identity-runtime.md) |
| 插件宿主 | [能力与插件执行](components/plugin-runtime.md) |
| 明文采集与协议恢复 | [TLS sync](components/tls-sync-runtime.md)、[流式解析器](components/streaming-parser.md) |
| eBPF | [事件 ABI](components/ebpf-event-abi.md)、[事件传输](components/ebpf-event-transport.md) |
| LLM 与工具语义 | [LLM Request](components/llm-request-projection.md)、[Live Tool](components/live-tool-projector.md) |
| MCP stdio | [MCP 观测](components/mcp-stdio-observation.md) |
| Hand-side | [执行隔离](components/execution-isolation.md) |
| Web | [Web Runtime](components/web-runtime.md)、[前端设计](components/web-frontend/README.md) |
| 资源观测 | [资源设计与组件图](components/resource-profiling/design.md) |

观测、治理、资源、隔离观测和告警投递的路径见[关键数据流](system/key-data-flows.md)。

## 部署视图

| 视图 | 架构说明 | 部署操作 |
| --- | --- | --- |
| 默认单主机 | [进程与存储位置](deployment/default.md) | [Host 部署](../operations/deployment/host.md) |
| 执行隔离 | [Host、VMM、Guest 与可信 daemon](deployment/execution-isolation.md) | [部署指南](../operations/deployment/execution-isolation.md) |
