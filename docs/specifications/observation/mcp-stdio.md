# MCP stdio 观测规范

> 本文定义本地 MCP stdio 会话的准入、动作图、内容留存和收口行为。

Status: Implemented
Owner: eBPF IPC lineage、MCP stdio session runtime 与 semantic projector
Scope: trace 内通过匿名 pipe 或 `AF_UNIX` socketpair 连接的本地 JSON-RPC 2.0 MCP server

## 术语

- **stdio bundle**：server exec 后，fd 0 与 fd 1 已证明连接到同一祖先 client 的通道集合。
- **Candidate**：已经通过 bundle 准入，但尚未观察到合法 `tools/call` 的有界协议候选。
- **Session**：至少观察到一条合法 client-to-server `tools/call` 的已确认连接。
- **canonical JSON**：object key 递归排序、array 顺序不变的 JSON-RPC object 表示。

## 会话准入

实现必须同时满足以下条件后才建立 Candidate：

1. server 已发生 exec；
2. fd 0 是 server 可读 endpoint，fd 1 是 server 可写 endpoint；
3. 两个 endpoint 的 peer owner 属于同一个祖先 client；
4. bundle 包含非空 identity、有效的 pipe 或 Unix socket channel kind，以及非零 client PID、generation 和 exec time；
5. lineage 未因当前 trace 的容量限制而停用。

只有 server fd 0 上的 inbound 消息同时满足以下条件，Candidate 才能确认 Session：

- `jsonrpc` 精确为 `"2.0"`；
- `method` 精确为 `"tools/call"`；
- `id` 是 string 或 number；
- `params.name` 是非空 string；
- `params.arguments` 缺省或为 object。

`initialize`、`tools/list`、通知、普通 JSON 和 server-to-client 输出不得单独确认 Session。framer 必须接受 JSON Lines、`Content-Length` 与 JSON-RPC batch，并把 batch 中的每个 JSON-RPC object 独立处理。

## 身份与生命周期

Session key 必须包含 trace id、stdin channel id 和 stdout channel id。process alias、fork、exec 或 FD 复制不得改变同一 channel 对的连接身份；channel 对改变时必须隔离新旧 Session。

进程身份必须包含非零 generation，防止 PID 复用。只有最后一个 bundle alias 关闭、进程退出或 trace finalize 时，才能清理连接状态。Candidate 不使用固定时间窗口淘汰合法的空闲连接；容量、framing 错误、截断、连接关闭和 trace 结束仍可终止它。

## 动作与关联

合法 request 必须创建 `mcp.tool_call`、`mcp.request` 和 `mcp.stdout`；匹配 response 必须创建 `mcp.response` 和 `mcp.stdin`。层级 link 必须形成：

```text
command.invocation
└── mcp.tool_call
    ├── mcp.request
    │   └── mcp.stdout
    └── mcp.response
        └── mcp.stdin
```

`mcp.stdout` 与 `mcp.stdin` 按 MCP client 视角命名。内部 link 与 command parent link 必须标记为 observed、valid。

request 与 response 必须按稳定 Session 和 request id 关联。id 重用时，invocation sequence 必须使 action id 保持唯一，待响应调用按 FIFO 匹配。JSON-RPC `error` 或 `result.isError=true` 必须生成 error/complete 终态；其他匹配 result 生成 success/complete。未匹配 response 不得生成孤立 response 分支。

LLM attribution 不得成为会话准入条件。存在匹配 proposal 时，可以通过 `llm.response.action_id`、`llm.tool_call.id` 和 `llm.tool_call.name` 关联 MCP root；无匹配时仍必须保留 MCP 动作图。

## 留存

原始 stdio storage mode 不得改变 Candidate 和 Session 的语义解析资格：

- `full` 可以保存 body 并引用已落盘 segment；
- `metadata-only` 可以保存 segment metadata 与 evidence，但不保存 body；
- `drop` 不保存 segment，也不得生成指向该 segment 的 evidence。

`semantic_retention.l0_mcp_call` 必须独立控制 request 和 response canonical JSON。开启时，内容只关联到 `mcp.request` 或 `mcp.response`，在单个 trace 内按 SHA-256 去重；stdio leaf 可以解引用其 parent 内容。canonical JSON 不得写入 OTEL span。

## 截断、错误与收口

- Candidate 的 stdin 截断必须拒绝该 Candidate；stdout 截断只重置 stdout framer。
- 已确认 Session 的非法、截断或超限消息只重置当前 stream buffer，不得撤销 Session。
- syscall payload 必须按真实返回长度组装；失败或零长度操作不得生成成功 payload。
- trace finalize 时，仍在 correlation state 的未响应 root 必须收口为 error/partial，不得伪造 response action。
- bundle 已关闭并清理 correlation state 后，不得为此前的 root 伪造终态。
- 单个 Candidate、Session、持久化或导出故障不得中断其他 trace 的采集。

当前组件关系及架构说明见 [MCP stdio 观测](../../architecture/components/mcp-stdio-observation.md)。
