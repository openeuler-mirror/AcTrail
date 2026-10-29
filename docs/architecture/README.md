# 架构设计

AcTrail 以本地 daemon 为观测与治理中心，围绕稳定进程身份、trace 生命周期和独立的使用方构建运行时。架构文档面向维护者和贡献者，说明当前职责、状态与交互；行为要求见[规范](../specifications/README.md)，长期技术选择见[架构决策](../decisions/README.md)。

## 阅读顺序

| 文档 | 回答的问题 |
| --- | --- |
| [架构图](c4-graphs.md) | 系统与外部如何交互，进程和组件如何部署 |
| [运行时序](sequence-graphs.md) | launch、观测、治理、存储及查询如何协作 |
| [代码布局](code-layout.html) | 可逐层展开的代码职责与四种实现状态；[布局源文件](code-layout.md) |
| [文档布局](doc-layout.md) | 文档目录与文件职责 |
| [配置布局](config-layout.md) | 配置归哪个模块解析，字段和默认值在哪里查询 |

## 系统与部署

| 主题 | 文档 |
| --- | --- |
| 系统边界 | [系统上下文](system/context.md)、[产品版图](system/product-landscape.md) |
| 进程与数据流 | [运行时容器](system/containers.md)、[关键数据流](system/key-data-flows.md) |
| 部署位置 | [默认部署](deployment/default.md)、[执行隔离部署](deployment/execution-isolation.md) |

## 组件设计

| 领域 | 组件文档 |
| --- | --- |
| daemon 与身份 | [Core Runtime](components/core-runtime.md)、[进程身份运行时](components/process-identity-runtime.md) |
| 内核采集 | [eBPF 源码布局](code-layouts/crates/adapters/collectors/ebpf/bpf/code-layout.md)、[事件 ABI](components/ebpf-event-abi.md)、[事件传输](components/ebpf-event-transport.md)、[探针检测器](components/probe-detector.md) |
| 明文与协议 | [TLS sync](components/tls-sync-runtime.md)、[流式解析器](components/streaming-parser.md)、[HTTP 内容解码](code-layouts/crates/core/semantic_action_runtime/src/llm_pipeline/transport/http1/code-layout.md)、[MCP stdio](components/mcp-stdio-observation.md) |
| 语义投影 | [LLM Request](components/llm-request-projection.md)、[Live Tool Projector](components/live-tool-projector.md)、[LLM Trajectory 图](components/llm-trajectory-graph.md) |
| 资源观测 | [资源设计](components/resource-profiling/design.md)、[样本存储](components/resource-profiling/storage.md)、[代码布局](code-layouts/crates/core/resource_profiling/code-layout.md)、[配置说明](components/resource-profiling/configuration.md) |
| 插件与告警 | [插件运行时](components/plugin-runtime.md)、[告警代理](components/alert-proxy.md) |
| 执行隔离 | [隔离运行时](components/execution-isolation.md)、[隔离源码布局](components/execution-isolation.md#源码职责与依赖) |
| 查询与界面 | [Web Runtime](components/web-runtime.md)、[Web 前端](components/web-frontend/README.md) |

资源观测的部署和验证入口见[Host 与 Workload](../operations/resource-metrics-host.md)、[已有容器](../operations/resource-metrics-containers.md)和[Guest](../operations/resource-metrics-sandboxes.md)。
