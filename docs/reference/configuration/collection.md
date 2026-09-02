# 采集与内容保留配置参考

> 本文说明采集能力、payload、语义保留和主动治理配置之间的关系。

采集配置分为能力契约、证据采集、语义保留和输出边界。各层有独立的 enable、容量和内容策略；打开上层不会自动放宽下层边界。

```mermaid
flowchart LR
    Contract["[capture]<br/>必需能力契约"] --> Collectors["eBPF / seccomp / TLS sync<br/>采集原始证据"]
    Collectors --> Payload["[payload.*]<br/>明文与分段容量"]
    Payload --> Semantic["[semantic_retention]<br/>HTTP / SSE / LLM / MCP 内容"]
    Semantic --> Storage["SQLite"]
    Semantic --> Export["Snapshot / OTEL exporter"]
    Limits["每层独立：enabled、容量、redaction、retention"] -.约束.-> Collectors
    Limits -.约束.-> Payload
    Limits -.约束.-> Semantic
    Limits -.约束.-> Export
```

## `[capture]`

`profile_name` 标识配置意图；`capabilities` 是每条 trace 必须满足的能力契约。当前 full-monitor 模板包含 process lifecycle/exec、file、mmap、network、IPC、stdio、TLS/socket plaintext、HTTP/HTTP2、resource metrics，以及 fanotify 文件治理能力。命令治理和 seccomp-notify 默认关闭，必须显式启用。

`agent_descendant_observation_depth` 控制 Agent 被识别后新建后代的详细采集深度。默认 `-1` 表示不限制；`0` 表示 Agent 保持详细采集、其未来后代仅保留生命周期；正数表示继续详细采集对应层数的未来后代。识别前已经存在的进程保持完整采集，生命周期和治理不受该选项影响。

Capability 保留在 required 列表、但提供该能力的 collector 被关闭时，配置或启动会失败。选择性能力应使用模板支持的 opportunistic/disabled 机制；拼写错误或字段缺失不能作为隐式降级手段。

## `[ebpf]`

`enabled` 控制主机 eBPF collector；map entry、ring buffer 和 path byte 上限约束内核与 daemon 资源。`file_path_capture_enabled` 决定是否保留路径事件。`[ebpf.ipc_lineage]` 关闭后不能继续把 IPC capability 声明为 required。

`preflight_link_teardown_workers` 有效范围为 `1..=16`，当前默认 `4`；worker 会在 readiness 前全部 join，不会跳过 preflight 或遗留 hook。

## `[payload.tls]`

当前默认启用 `tls-sync`，provider/source/resolver/library 为 `auto`，runtime library path 为 `auto`，event socket 为 `/run/actrail/tls-sync.sock`。主要边界：

- `max_segment_bytes`：单个 inline segment 上限；
- `max_operation_bytes`：一次 operation 可读取上限；
- `ring_buffer_bytes`、`pending_operation_max_entries`：运行中容量；
- `retention_max_bytes_per_trace`：每 trace 持久化上限；
- `redaction_policy`：写入前的内容 redaction，当前默认 `disabled`；
- `java_agent_enabled`：仅 Java JSSE workload 需要，默认 `false`。

TLS sync 必须使用 `actrailctl launch`。resolver 无法为实际 binary 生成完整 plan 时不能回退为“已捕获 TLS 明文”。

## `[payload.socket]`、`[payload.stdio]` 与 `[payload.mcp]`

本节帮助部署者选择 socket 明文采集后端，并理解两种模式的完整性保证。Socket 监听 `write`、`writev`、`sendto`、`sendmsg`，由 `payload.socket.capture_backend` 选择采集路径：

- `bpf-copy`（默认，性能/证据模式）：仅依赖 eBPF，不要求 seccomp notify 或 `actrailctl launch`。每次 operation 最多抓取前导的一个 `max_segment_bytes` segment（默认 4095 字节；`writev`/`sendmsg` 取第一个非空 iovec 的头部）。超出部分标记为 payload `PolicyLimited`；可信 HTTP 路由仍可生成不伪造正文的 `llm.request`，其 status 为 `success`、completeness 为 `capture_limited`，并可与完整 response 关联用于性能剖析。这是该模式的预期结果，不产生截断错误 diagnostic。
- `bpf-copy-seccomp-fallback`（完整采集，需显式启用）：要求 `[seccomp_notify] enabled = true`，workload 必须通过能安装 seccomp listener 的路径启动（例如 `actrailctl launch` 或容器 seccomp profile）。该模式下 BPF 只产生 operation 完成元数据（completion/sequence）；需要内容字节的 operation 由 daemon 在 seccomp notify 上读取并切分，完整上限为 `max_operation_bytes`，适合需要完整 HTTP/LLM 消息的观测。读取失败、缺口或不完整 operation 标记为 payload `Truncated` 和 semantic `partial`，属于异常并产生 diagnostic。未启用 seccomp notify 时配置校验会直接失败，不会静默降级。

三类 payload 的 ring buffer、pending state、每 trace retention 与 redaction 均独立。调高其中一层不会自动扩大其他层。

Stdio 的 stdin/stdout/stderr 分别有 capture 和 storage mode；当前模板会完整保留 stdin、丢弃 stdout body、仅保留 stderr metadata。MCP 配置限制 parse buffer 与候选状态容量。

## `[semantic_retention]`

当前默认 `content_owner = "highest_consumed"`：内容被更高语义层消费后，低层只保留摘要、计数、transport metadata 与 evidence reference，避免重复持有同一 body。

| 层 | 当前默认重点 |
| --- | --- |
| `l0_llm_call` | 启用；request `canonical_blocks`；request body export `none`；response `assembled_provider` |
| `l0_mcp_call` | request/response `canonical_json` |
| `l1_sse` | 保留 stream summary，不保留 event content |
| `l2_http` | 保留 message summary、header metadata 和 body text |
| `l3_http2_frame` | 保留 frame summary，不保留 DATA content |
| `l4_payload` | 当前关闭 body retention，只保留 stats |

Capacity exhaustion、明确 `Truncated` 或 partial operation 只隔离受影响的 direction/stream，并产生 diagnostic；在重新观察到可信 message boundary 前不能把后续字节错误关联为完整请求。`PolicyLimited` 同样隔离缺失字节，但作为性能模式的预期边界静默恢复。

## 治理配置

`[enforcement]`、`[command_control]` 和 `[network_control]` 会改变工作负载行为，不只是采集。当前生成配置保留 fanotify 文件控制，但 seccomp syscall 列表为空，命令控制和网络控制关闭。需要同步治理时，部署必须显式启用 seccomp-notify 及对应控制能力，并审查规则文件、default/failure decision、gray timeout/fallback、审计和 capability 组合。
