# AcTrail Documentation

This directory is the operator-facing documentation entrypoint. Run commands from the repository root unless a document says otherwise.

## Start Here

| Document | Use It For |
| --- | --- |
| [Usage Guide](usage.md) | Daily commands: build, preflight, clean, start, attach or launch, inspect, export, stop. |
| [Deployment Guide](deployment.md) | Host prerequisites, config layout, daemon operation, storage/log handling, and transfer-test readiness. |
| [Containerized Agent Deployment](containerized-agent-deployment.md) | Host daemon, not containerized, plus one Docker workload container, including required mounts, permissions, xiaoO launch, and host-side verification. |
| [Use Cases](use-cases.md) | Which example to run for process trees, LLM payloads, HTTP semantics, enforcement, OTEL export, and performance checks. |
| [LLM Request Canonical Blocks](llm-request-canonical-blocks.md) | L0 request-body retention model for reconstructable, trace-local deduplicated LLM request content. |
| [Platform Requirements](platform-requirements.md) | Kernel, tracefs, BTF, seccomp, pidfd, fanotify, architecture, and failure interpretation. |
| [Examples Checklist](examples/TESTING.md) | QA handoff checklist for examples under `docs/examples/` and the real agent trace suite under `tests/agent-trace/`. |
| [Preflight Tools](preflight/README.md) | One-command platform scan and local smoke run. |

## Current Runtime Model

AcTrail is config-driven and fail-fast. Required capabilities are declared in an operator config; if the host cannot provide them, the run should fail instead of silently downgrading coverage.

The main runtime flow is:

```text
actraild -> collectors/analyzers -> AcTrail storage -> actrailviewer/actrailweb/export
```

Use `actrailctl track-add` for an already-running process when you only need observation. Use `actrailctl launch` when AcTrail must prepare the child before `exec`, such as TLS sync payload capture (`LD_PRELOAD` runtime plus finder fast probe plan), socket large-payload seccomp fallback, or process seccomp agent-invocation observation.

Process fork observation in the eBPF collector uses `sched/sched_process_fork`. The syscall tracepoint `syscalls/sys_enter_fork` is not required.

For real agent acceptance, use `tests/agent-trace/` after the release binaries are built. That suite runs Claude Code, opencode, xiaoO, and a LangGraph Python workload through AcTrail and validates retained payloads, semantic actions, and OTEL spans for the actions each case declares.
