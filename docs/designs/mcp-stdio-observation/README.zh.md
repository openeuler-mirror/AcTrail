# MCP stdio 工具调用观测文档

本目录记录本地 stdio MCP 工具调用观测实现。

## 阅读入口

1. [实现流程、配置与语义模型](implementation-flow.zh.md)
2. [Claude MCP 真实 Agent 回归测例](../../../tests/v2/regression/probe_claude_mcp/README.zh.md)
3. [Codex MCP 真实 Agent 回归测例](../../../tests/v2/regression/probe_codex_mcp/README.zh.md)

## 当前范围

- 支持由被观测 Agent 通过匿名 pipe 或 `AF_UNIX` `socketpair` 启动的本地 stdio
  MCP server。
- 支持 JSON Lines 与 `Content-Length` 两种 JSON-RPC 2.0 framing。
- 只在观察到合法的客户端到服务端 `tools/call` 后确认 MCP 会话，普通 stdio
  不进入 MCP 语义投影。
- 生成 `mcp.tool_call`、`mcp.request`、`mcp.response`、`mcp.stdout` 和
  `mcp.stdin` 语义动作，并接入 SQLite、Web action tree 和 OTEL 导出。
- 可独立于原始 stdio payload 留存策略，把 `tools/call` 请求与响应保存为规范化
  JSON-RPC 内容；Web detail panel 优先读取这份语义内容。

## 重要留存边界

`payload.stdio.*_storage_mode` 只控制原始 stdio payload 和对应 evidence。默认
`stdout_storage_mode="drop"` 不代表 MCP 响应内容不会落盘：
`semantic_retention.l0_mcp_call.response_content` 默认为 `"canonical_json"`，仍会把
完整 JSON-RPC 响应写入 SQLite。处理敏感工具参数或结果时，必须同时检查这两层配置。
