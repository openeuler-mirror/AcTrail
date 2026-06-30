# AcTrail Examples

Use [08.full-monitor-validation](08.full-monitor-validation/README.md) for real agent end-to-end validation with `actrailctl launch`, semantic actions, payloads, OTEL export, and `actrailweb`.

## Index

| Directory | Entry |
| --- | --- |
| `01.quick-start` | [README](01.quick-start/README.md) |
| `02.llm-http-payload-capture` | [README](02.llm-http-payload-capture/README.md) |
| `03.extended-observation-e2e` | [README](03.extended-observation-e2e/README.md) |
| `04.fanotify-enforcement-e2e` | [README](04.fanotify-enforcement-e2e/README.md) |
| `05.http-payload-unified` | [README](05.http-payload-unified/README.md) |
| `06.xiaoo-tls-capture` | [README](06.xiaoo-tls-capture/README.md) |
| `07.xiaoo-claude-agent-invocation` | [README](07.xiaoo-claude-agent-invocation/README.md) |
| `08.full-monitor-validation` | [README](08.full-monitor-validation/README.md) |
| `09.agentscope-http-https` | [README](09.agentscope-http-https/README.md) |
| `10.java-langchain4j-agent` | [README](10.java-langchain4j-agent/README.md) |
| `container-agent-minimal` | [README](container-agent-minimal/README.md) — host daemon baseline config for container-side `actrailctl launch`. |
| `container-agent-restricted` | [README](container-agent-restricted/README.md) — degradation test: container with default seccomp and no eBPF/CAP_BPF, proving probe/launch auto-degrade and tls-sync-only capture. |

Regression coverage for documented examples is described in [TESTING.md](TESTING.md).
