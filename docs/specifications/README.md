# 规范

> 本文说明 AcTrail contract 的必需行为、负责人、范围和实现状态。

每份规范先声明状态、负责人和范围，再定义实现必须满足的行为与约束。

- 观测：[沙箱观测](observation/sandbox-observation.md)、[MCP stdio](observation/mcp-stdio.md)、[流式解析器](observation/streaming-parser.md)和[探针检测器](observation/probe-detector.md)
- 身份：[launch trace 注册](identity/launch-trace-registration.md)
- 执行隔离：[运行时生命周期](isolation/runtime-lifecycle.md)和[告警交付](isolation/alert-delivery.md)
- 导出：[动作交付](export/action-delivery.md)和[File I/O 终态动作](export/file-io-terminal-actions.md)
- [插件](plugins/)
