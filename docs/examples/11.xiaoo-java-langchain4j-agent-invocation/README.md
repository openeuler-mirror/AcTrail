# xiaoO exec Java LangChain4j Agent 的缺陷暴露用例

这个示例验证真实 xiaoO agent exec 拉起一个 Java 框架 Agent 时，AcTrail 能否在 Java child 的真实 HTTPS LLM 调用之后采集 JSSE payload，生成 `llm.request`，并把对应 `command.invocation` 标注为 agent 调用。

这个 case 复用 case07 的外层形态：runner 先在 AcTrail 启动前构建 fat jar，再通过 `actrailctl launch -- xiaoo run -p ...` 启动 xiaoO。xiaoO 必须执行一条 `timeout ... java -jar ...` 命令。非 launch 指的是 Java 框架 AI Agent：它不是 `actrailctl launch` 的直接 target，而是 xiaoO 通过普通 exec 拉起的 child。

这个 case 的目的不是改写 Java framework AI Agent 的启动命令，而是验证 Java child exec 场景可以通过 trace-scoped 环境注入捕获 JSSE 明文。`operator.conf` 保留 `agent_invocation_command = java`，并设置 `payload_tls_java_agent_enabled = true`：`actrailctl launch` 只把 `JAVA_TOOL_OPTIONS` 和 TLS sync env 注入到 xiaoO 这个 traced root 环境，Java child 通过普通 exec 继承 JVM 原生支持的 agent transport；AcTrail 不改写 Java child 的 argv。导出的 OTEL 必须包含 xiaoO `process.exec`、Java child `process.exec`、同 Java PID 的完整 `llm.request`，以及 child 为 Java、`invocation.kind=agent` 且 evidence 指向该 `llm.request` 的 `command.invocation`。

当前通过条件要求 Java child 的 HTTPS/JSSE payload 来自正常 xiaoO child exec，而不是把 `java -jar ...` 作为 `actrailctl launch` 的直接 target，也不是手工改写 Java argv。

## 文件

| 文件 | 用途 |
| --- | --- |
| `operator.conf` | AcTrail operator 配置，用于 process exec observation、JSSE payload capture、stdio capture 和 agent identity 语义组装；socket plaintext fallback 被禁用。 |
| `workload.conf` | xiaoO、共享 Java workload、HTTPS provider、prompt、launch timeout、drain 和 OTEL 输出参数。 |
| `agent_prompt.template` | 提示词模板，要求 xiaoO 只执行一次前台 `java -jar ...` 命令。 |
| `run_e2e.py` | 构建 Java fat jar，按 case07 方式 launch xiaoO，然后验证 xiaoO exec 出来的 Java child payload、semantic action 和 OTEL agent edge。 |
| `../_workloads/java-langchain4j-agent/` | 非编号共享 workload；10 和 11 都用它运行真实 LangChain4j `OpenAiChatModel`。 |

## 前置条件

- 在仓库根目录用 root shell 运行。
- 先用 JDK 17+ 在 `PATH` 中完成 release 构建：`cargo build --release`。
- `xiaoo` 在 `PATH` 中；`which xiaoo` 应打印将要启动的可执行文件。
- JDK 17+ 的 `java`、`javac` 和 Maven `mvn` 在 `PATH` 中。
- 设置 `DEEPSEEK_API_KEY`，或使用 `ACTRAIL_LLM_API_KEY_ENV`、`ACTRAIL_LLM_BASE_URL`、`ACTRAIL_LLM_CHAT_PATH`、`ACTRAIL_LLM_MODEL` 覆盖 provider。
- provider 必须是 HTTPS。不要改成 plain HTTP 或本地 socket fallback，否则会绕开本 case 要暴露的 Java HTTPS/JSSE 缺陷。
- xiaoO 和 Java workload 都能访问外部网络。

## 运行

```bash
python3 docs/examples/clean.py --example xiaoo-java-langchain4j-agent
python3 docs/examples/11.xiaoo-java-langchain4j-agent-invocation/run_e2e.py
```

期望终端输出包含：

```text
ACTRAIL_LANGCHAIN4J_AGENT_COMPLETE
xiaoo_java_langchain4j_trace_id=<TRACE_ID>
xiaoO Java LangChain4j agent invocation docs e2e complete
```

上述输出是通过条件。runner 会导出 `/tmp/actrail-xiaoo-java-langchain4j-agent-invocation.otlp.json`，并验证：

- xiaoO 的 `process.exec` 被观测到。
- Java child 的 `process.exec` command line 包含 `java -jar` 和共享 workload 的 `java-langchain4j-agent-0.1.0-all.jar`。
- 同一 Java PID 上存在来自 JSSE/TlsUserSpace 的完整成功 `llm.request`，并且 span 里包含配置的 model 和 prompt。
- 存在 `invocation.kind=agent` 的 `command.invocation`，其 `agent.child.pid` 等于 Java PID。
- `agent.invocation.evidence_action_id` 指向 Java child 的 `llm.request` action id。
- xiaoO exec、Java exec 和 agent command span 处于同一 trace。

## Provider Overrides

覆盖变量和 `10.java-langchain4j-agent` 保持一致：

```bash
ACTRAIL_LLM_BASE_URL=https://api.deepseek.com
ACTRAIL_LLM_CHAT_PATH=/chat/completions
ACTRAIL_LLM_MODEL=deepseek-chat
ACTRAIL_LLM_API_KEY_ENV=DEEPSEEK_API_KEY
ACTRAIL_LLM_PROMPT='Reply exactly with ACTRAIL_XIAOO_JAVA_LANGCHAIN4J_OK'
```

如果覆盖 prompt 且仍需要严格校验回答 marker，同时设置 `ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT`。否则 runner 只要求 Java workload 返回非空 LLM answer。`ACTRAIL_LLM_BASE_URL` 必须保持 HTTPS。

## 手动检查

缺陷修复并成功运行后，可以检查 agent-labeled Java command：

```bash
jq '[.resourceSpans[].scopeSpans[].spans[] | select(any(.attributes[]?; .key=="actrail.action.kind" and .value.stringValue=="command.invocation") and any(.attributes[]?; .key=="invocation.kind" and .value.stringValue=="agent") and any(.attributes[]?; .key=="agent.child.command_line" and (.value.stringValue | contains("java-langchain4j-agent-0.1.0-all.jar"))))] | length' \
  /tmp/actrail-xiaoo-java-langchain4j-agent-invocation.otlp.json
```

输出至少应为 `1`。

这个示例不校验 xiaoO 自己的自然语言决策质量，也不要求 xiaoO 自己的 LLM request 一定能被完整投影；通过条件绑定到 xiaoO 拉起的 Java child。禁止启用 socket fallback、手工注入 `JAVA_TOOL_OPTIONS` 或把 Java child 改成 `actrailctl launch` 直接 target 来让用例通过。
