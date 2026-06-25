# AcTrail Tests

本表记录 `tests/` 下主要测例的运行入口、测试目标和预期现象。默认先构建 release 产物：

```bash
cargo build --release
```

除特别说明外，E2E 测例需要在仓库根目录运行，并依赖真实 AcTrail release binaries。真实 agent/provider 测例还需要对应 CLI、凭据和网络环境；依赖缺失时 regression runner 会按测例策略 `SKIP` 或 fail-fast。

| 测例项 | 运行指令 | 测试目标 | 预期现象 |
| --- | --- | --- | --- |
| Regression quick suite | `python3 tests/regression/test_all.py` | 按 quick suite 汇总运行核心平台、agent、payload、enforcement 和 docs transfer 检查。 | 输出每个 case 的 PASS/SKIP/FAIL 汇总，并在 `/tmp/actrail-regression-*` 下生成 Markdown/JSON 报告。 |
| Regression case list | `python3 tests/regression/test_all.py --list` | 列出 regression runner 发现的所有 case、suite 归属和标题。 | 只打印 case 清单，不启动 daemon 或真实 workload。 |
| `initialize` | `python3 tests/regression/test_all.py --case initialize` | 检查运行平台、root 权限、release binaries、seccomp user notification、BTF、tracefs、platform preflight。 | 必要能力存在时 PASS；缺 root、BTF、tracefs 或 release binary 时明确 FAIL/WARN。 |
| `e2e-claude` | `python3 tests/regression/test_all.py --case e2e-claude` | 先验证 `claude -p` 可用，再通过 `tests/payload/claude-code/run_e2e.py` 捕获真实 Claude Code LLM exchange。 | 看到 Claude 可用性 marker；payload rows、`llm.request`/`llm.response`/`llm.call`、web action tree、OTEL semantic spans 均完整。 |
|  |  | 验证 TLS/plain HTTP source 选择和 LLM payload retention。 | payload export 保留元数据但不保留 LLM 正文；OTEL `llm.request` canonical preview 包含 marker；原始敏感 body 不直接打印。 |
| `e2e-opencode` | `python3 tests/regression/test_all.py --case e2e-opencode` | 捕获真实 opencode CLI 的 provider LLM exchange，并验证自动 TLS probe plan。 | opencode 进程产生完整 payload rows、LLM semantic graph、web tree 和 OTEL request/response spans。 |
| `e2e-xiaoo` | `python3 tests/regression/test_all.py --case e2e-xiaoo` | 验证新版 `xiaoo --cli run` 可用，并捕获 xiaoO rustls HTTPS LLM exchange。 | xiaoO availability marker 出现；`tls-probe-point-finder fast` 找到 rustls plaintext plan；payload、LLM actions、OTEL spans 完整。 |
| `e2e-langgraph` | `python3 tests/regression/test_all.py --case e2e-langgraph` | 使用真实 LangGraph Python workload 调 OpenAI-compatible provider，覆盖 Python `_ssl`/OpenSSL attach 路径。 | 选中的 Python 能 import `langgraph`/`requests` 且满足 TLS 要求；LangGraph 产生完整 LLM semantic exchange 和 OTEL spans。 |
| `enforcement-fanotify` | `python3 tests/regression/test_all.py --case enforcement-fanotify` | 包装 fanotify enforcement docs E2E，验证文件访问 allow/deny 决策。 | allowed read 成功、denied read 返回 permission denied；OTEL export 中包含 allow 和 deny enforcement spans。 |
| `http-payload` | `python3 tests/regression/test_all.py --case http-payload` | 包装 HTTP payload docs E2E，验证 plain HTTP socket payload 和 HTTP application semantics。 | viewer 显示 `POST /plain-http` application request；payload source 包含 `Syscall/socket-syscall`。 |
| `http-llm-projection` | `python3 tests/regression/test_all.py --case http-llm-projection` | 本地 plain HTTP OpenAI-style request 被 socket payload 投影为 `llm.request`。 | pretty OTEL 中出现 `llm.request` span；request payload/model/messages 语义字段完整。 |
| `docs-examples` | `python3 tests/regression/test_all.py --case docs-examples` | 回放文档中的 quick-start、HTTP/2 local、外部 OpenAI-compatible HTTP/1.1/HTTP/2、extended observation、xiaoO -> Claude agent invocation。 | 每个文档步骤输出 expected/found evidence；缺外部 API key 的 provider 步骤 SKIP；xiaoO -> Claude 步骤验证 agent edge、Claude exec/LLM request、Claude 子 Bash command。 |
| `e2e-xiaoo-http-proxy` | `python3 tests/regression/test_all.py --case e2e-xiaoo-http-proxy` | 真实 xiaoO 通过本地 plain HTTP OpenAI-compatible provider shim 发起 LLM exchange。 | local-stream 模式无需上游 key；plain socket payload rows 完整；`llm.call`/`llm.request`/`llm.response` action graph 成功且没有 failed `llm.response`。 |
| Agent trace: Claude Code | `python3 tests/agent-trace/run_case.py claude-code` | 直接运行 Claude Code payload E2E，绕过 regression 包装层。 | 输出 `claude code LLM payload e2e complete`、trace id、payload segment 数、web tree reachable 数和 OTEL span 数。 |
| Agent trace: opencode Bun | `python3 tests/agent-trace/run_case.py opencode-bun` | 真实 opencode/Bun provider traffic 的 payload 与 semantic capture。 | 自动 TLS plan 可用；payload rows、LLM exchange、web tree、OTEL request/response spans 完整。 |
| Agent trace: xiaoO rustls | `python3 tests/agent-trace/run_case.py xiaoo-rustls` | 真实 xiaoO rustls HTTPS plaintext capture，覆盖新版 `xiaoo --cli run`。 | 输出 `ACTRAIL_XIAOO_OK`；evidence source 为 `TlsUserSpace`；`xiaoo_llm_request_spans` 和 `xiaoo_llm_response_spans` 均非零。 |
| Agent trace: xiaoO HTTP proxy | `python3 tests/agent-trace/run_case.py xiaoo-http-proxy` | 启动本地 provider shim，让 xiaoO 走 plain HTTP provider path。 | 输出 `ACTRAIL_XIAOO_HTTP_PROXY_OK`；payload source 为 `Syscall/socket-syscall`；HTTP/LLM semantic actions 完整。 |
| Agent trace: LangGraph OpenAI | `python3 tests/agent-trace/run_case.py langgraph-openai` | 真实 LangGraph Python agent 的 OpenAI-compatible LLM exchange。 | LangGraph workload 输出 marker；捕获 request/response payloads、LLM actions 和 OTEL spans。 |
| Agent trace: Go net/http | `python3 tests/agent-trace/run_case.py go-net-http` | 构建并运行 Go `net/http` workloads，验证 Go TLS/HTTP 自动解析。 | 标准库和 wrapper workload 都能被捕获；Go TLS probe 自动解析，不需要手写 library/binary path。 |
| Agent trace: Java Netty tcnative | `python3 tests/agent-trace/run_case.py java-netty-tcnative` | Java Netty/tcnative TLS provider traffic 的 payload capture。 | Java workload 产生完整 payload rows 和 LLM semantic/OTEL exchange。 |
| Agent trace: dynamic TLS | `python3 tests/agent-trace/run_case.py dynamic-tls` | 验证 OpenSSL `DT_NEEDED` 和 `dlsym` 动态 TLS runtime/probe 解析路径。 | runtime 能发现并 attach 所需 TLS plaintext hook；payload evidence 可被 viewer/OTEL 使用。 |
| Claude payload direct | `python3 tests/payload/claude-code/run_e2e.py` | 直接捕获真实 Claude Code LLM payload、semantic graph、web action tree 和 OTEL export。 | 输出 `claude code LLM payload e2e complete`；payload export 保留元数据但不保留 LLM 正文；OTEL `llm.request` canonical preview 包含 prompt marker；response payload rows 满足 TLS/plain HTTP source 策略。 |
| HTTP payload direct | `python3 tests/payload/http-local/run_e2e.py` | 复用 docs HTTP payload runner 和测试专用 config，验证 local HTTP payload capture。 | local HTTP workload 完成；viewer 显示 application HTTP request 和 socket payload source。 |
| Fanotify enforcement direct | `python3 tests/enforcement/fanotify/run_e2e.py` | 直接生成临时 fanotify operator config，验证文件 permission enforcement。 | allowed path 读取成功，denied path 被拒绝；OTEL 中有 allow/deny spans。 |
| Agent invocation E2E | `python3 tests/process/agent-invocation/run_e2e.py` | 真实 xiaoO 调用前台 Claude；验证 agent identity 由 Claude 子进程 LLM evidence 推导，而非命令名硬编码。 | launch 输出 `ACTRAIL_AGENT_TREE_OK`；OTEL 中有 Claude `process.exec`、同 pid `llm.request`、Claude 直接子 Bash `command.invocation`、agent-labeled Claude `command.invocation`。 |
| Agent invocation probe: bare xiaoO | `python3 tests/process/agent-invocation/run_probe.py bare-xiaoo` | 不启动 AcTrail，单独验证 xiaoO prompt 是否能拉起 Claude 并返回 marker。 | 命令成功，输出 marker 和 `probe=bare-xiaoo elapsed_seconds=...`。 |
| Agent invocation probe: direct Claude | `python3 tests/process/agent-invocation/run_probe.py direct-claude` | 只在 AcTrail 下启动 Claude，排除 xiaoO 外层影响。 | Claude marker 出现，trace 能生成，用于定位 Claude 侧采集问题。 |
| Agent invocation probe: strace xiaoO | `python3 tests/process/agent-invocation/run_probe.py strace-xiaoo` | 用 strace 观察 xiaoO 是否 fork/exec Claude 及其 wrapper。 | 输出 strace syscall 汇总，命令成功并返回 marker。 |
| Hidden agent invocation | `python3 tests/process/hidden-agent-invocation/run_e2e.py` | 编译 `agent_a`，形成 `agent_a -> script_b.sh -> xiaoo --cli run`，验证隐藏 agent identity 和直接父子关系。 | `agent_a` 与 xiaoO 均有真实 LLM evidence；只记录 `script_b.sh -> xiaoo` 的 direct agent invocation，不生成 `agent_a -> xiaoo` 祖先快捷边。 |
| Concurrent launch: shell | `python3 tests/process/concurrent-launch/run_e2e.py --workload shell` | 在一个 daemon 下并发启动多个本地 shell workload，验证 active trace limit、trace lifecycle、stdout payload marker。 | 所有 trace 进入 Active/Clean 并完成；超过 active limit 的 track-add 被拒绝。 |
| Concurrent launch: xiaoO | `python3 tests/process/concurrent-launch/run_e2e.py --workload xiaoo --concurrency 2 --xiaoo-bin /root/projects/xiaoO/target/release/xiaoo` | 并发真实 xiaoO CLI workload，验证新版 `--cli run` 在多 trace 下的 capture 和 completion。 | 每个 xiaoO 输出对应 `ACTRAIL_XIAOO_N` marker；trace 完成且 outbound payload 中能查到 marker。 |
| File scan recording | `python3 tests/process/file-scan-recording/run_e2e.py` | 运行重复 `rg` 文件扫描，验证 canonical path set/chunk 复用，避免路径集合重复膨胀。 | trace 完成；SQLite 中 file path set/chunk 结构满足复用断言；输出 `file scan recording e2e passed`。 |
| Performance benchmark | `python3 tests/performance/run_benchmark.py --case all --mode all --output local/performance-benchmark.md` | 测量 baseline、daemon idle、eBPF core、payload、seccomp agent 等模式下的任务耗时分布。 | 生成 Markdown 报告，包含 median/p95、overhead、KS/Mann-Whitney/Hodges-Lehmann 统计和 raw timings。 |
| Performance single case | `python3 tests/performance/run_benchmark.py --case agent --mode baseline,observed-ebpf-payload --repetitions 30 --output local/performance-agent.md` | 对单个 workload/模式组合做可控重复测试。 | 报告只包含指定 case/mode；任一 run fail/timeout 时 benchmark 无效并 fail-fast。 |
| LLM HTTP proxy support smoke | `python3 tests/support/llm-http-proxy/provider_proxy.py --mode local-stream` | 启动本地 OpenAI-compatible SSE provider shim，供 xiaoO HTTP proxy 测例使用。 | 监听本地端口并返回 deterministic SSE；它是 support server，不单独证明 AcTrail capture。 |
