# 架构

> 本文说明当前运行时组件、关键数据流及其源码归属。

这些页面面向维护者和贡献者，描述 AcTrail 当前的实现。规范性行为由[规范](../specifications/README.md)定义。

- [系统上下文](system/context.md)
- [产品版图](system/product-landscape.md)
- [运行时容器](system/containers.md)
- [关键数据流](system/key-data-flows.md)
- [默认部署](deployment/default.md)
- [执行隔离部署](deployment/execution-isolation.md)
- 组件：[Core Runtime](components/core-runtime.md)、[进程身份运行时](components/process-identity-runtime.md)、[Web Runtime](components/web-runtime.md)、[Web 前端](components/web-frontend.md)、[执行隔离](components/execution-isolation.md)、[LLM Request 当前投影路径](components/llm-request-projection.md)、[Live Tool Projector](components/live-tool-projector.md)、[告警代理](components/alert-proxy.md)、[插件运行时](components/plugin-runtime.md)、[探针检测器](components/probe-detector.md)、[MCP stdio 观测](components/mcp-stdio-observation.md)、[eBPF 源码布局](components/ebpf-source-layout.md)、[eBPF 事件 ABI](components/ebpf-event-abi.md)、[eBPF 事件传输](components/ebpf-event-transport.md)、[TLS sync 运行时](components/tls-sync-runtime.md)和[流式解析器](components/streaming-parser.md)
- [代码布局](code-layout/README.md)
