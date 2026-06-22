# Java LangChain4j Agent

This example launches a real Java workload using LangChain4j `OpenAiChatModel`. It calls an OpenAI-compatible external LLM provider through the JDK HTTP client over HTTPS. The workload does not use a local stub, replay server, HTTP relay, custom HTTP client, or protocol downgrade. AcTrail observes this path by enabling its launch-time JSSE Java agent (`payload_tls_java_agent_enabled = true`), which reports HTTPS plaintext through the normal tls-sync event socket.

Defaults target DeepSeek:

- Base URL: `https://api.deepseek.com`
- Chat path: `/chat/completions`
- Model: `deepseek-chat`
- Key environment variable: `DEEPSEEK_API_KEY`

LangChain4j documents the plain Java Maven artifact as `dev.langchain4j:langchain4j-open-ai` and exposes `baseUrl`, `apiKey`, and `modelName` on `OpenAiChatModel`. The same docs state that the plain Java OpenAI integration uses the JDK `java.net.http.HttpClient` by default.

## Files

| File | Purpose |
| --- | --- |
| `../_workloads/java-langchain4j-agent/pom.xml` | Minimal Maven project with `dev.langchain4j:langchain4j-open-ai:1.16.1`; `mvn package` creates the executable fat jar `target/java-langchain4j-agent-0.1.0-all.jar`. |
| `../_workloads/java-langchain4j-agent/src/main/java/.../LangChain4jAgent.java` | Java workload that builds `OpenAiChatModel` in the ordinary LangChain4j style, sends one prompt, and prints the LLM answer marker. |
| `operator.conf` | AcTrail operator config with process/network capture, JSSE TLS plaintext payload, HTTP/1.x and HTTP/2 analyzers, payload text export, and semantic action export. |
| `workload.conf` | Provider, prompt, timeout, Maven package, drain, and OTEL output settings. |
| `run_e2e.py` | End-to-end runner that builds the fat executable jar, launches it with `java -jar`, and asserts payloads, semantic actions, and OTEL export. |

## Preconditions

- Run from the repository root in a Linux/WSL root shell.
- Build release binaries with JDK 17+ on `PATH`: `cargo build --release`.
- Set `DEEPSEEK_API_KEY`.
- JDK 17+ `java` and `javac` are on `PATH`, and `mvn --version` reports a Java 17+ runtime.
- External network access is available for Maven Central and the LLM provider.

The Maven package build runs before AcTrail starts so dependency downloads do not pollute the trace. The first run on a cold Maven cache can take several minutes while Maven Central dependencies are downloaded. The traced workload is the packaged deployment artifact, launched as `java -jar ...` from `docs/examples/_workloads/java-langchain4j-agent/target/`.

## Provider Overrides

The runner accepts these environment overrides:

```bash
ACTRAIL_LLM_BASE_URL=https://api.deepseek.com
ACTRAIL_LLM_CHAT_PATH=/chat/completions
ACTRAIL_LLM_MODEL=deepseek-chat
ACTRAIL_LLM_API_KEY_ENV=DEEPSEEK_API_KEY
ACTRAIL_LLM_PROMPT='Reply exactly with ACTRAIL_LANGCHAIN4J_DOCS_OK'
```

`ACTRAIL_LLM_CHAT_PATH` must end with `/chat/completions`, because LangChain4j configures the API base URL and appends that chat-completions route internally. `ACTRAIL_LLM_BASE_URL` must stay HTTPS; plain HTTP would avoid the JSSE path this case is meant to validate. If you override the prompt and still want a strict answer marker check, also set `ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT`. Without that extra variable, the runner only requires a non-empty real LLM answer.

## Run

```bash
python3 docs/examples/clean.py --example java-langchain4j-agent
python3 docs/examples/10.java-langchain4j-agent/run_e2e.py
```

Expected result:

- The traced workload command uses `java -jar` with the shared Java LangChain4j fat jar.
- The Java workload prints a non-empty `llm_answer=...` line and `ACTRAIL_LANGCHAIN4J_AGENT_COMPLETE`.
- `actrailviewer payloads` shows a complete successful outbound `TlsUserSpace/jsse` plaintext row.
- `actrailviewer actions` contains a complete successful `llm.request`.
- `export-otel` writes `/tmp/actrail-java-langchain4j-agent.otlp.json` with an `actrail.action.kind=llm.request` span containing the configured model and prompt text.

This case intentionally does not avoid Java JSSE HTTPS or force HTTP/1.1. If the real LLM call succeeds but AcTrail cannot project `llm.request`, the runner fails and reports that the configured JSSE Java-agent capture path is not producing the expected payload evidence.

`llm.response` OTEL evidence is printed when present, but it is not a required pass condition for this docs transfer test.
