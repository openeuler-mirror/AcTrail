# AcTrail

> AcTrail records the actual trail of AI agent actions.

AcTrail 是面向 Linux/WSL 的 AI agent 系统侧观测工具链。它从真实运行时捕获进程、网络、文件、IPC、payload、HTTP semantic events 和资源采样，把 raw runtime events 转成可信、跨框架的 agent action trail，并通过 `actrailviewer` 和只读 Web UI `actrailweb` 查看。

当前公开文档以配置文件驱动为主，不要求用户直接读取 storage，也不要求先导出 JSON 才能查看结果。

## Components

```text
actraild -> libbpf eBPF collector -> ingest/analyzers -> AcTrail storage -> actrailviewer/actrailweb
```

| Component | Role |
| --- | --- |
| `actraild` | 采集 daemon，负责 eBPF collector、资源 sampler、analyzer 生命周期、trace attach 和 storage 写入。 |
| `actrailctl` | 控制面 CLI，用于 doctor、launch、track-add、track-remove、list-traces、clean。 |
| `actrailviewer` | 只读 CLI viewer，通过配置里的 `storage_path` 查看 trace、事件、网络、payload、诊断和 JSON export。 |
| `actrailweb` | 只读 Web UI，通过配置里的 `storage_path` 展示 trace 列表、Timeline、Process Tree、Resources、事件、进程、payload 和诊断。 |

## Quick Start

Install the native build dependencies first. On openEuler/Fedora/RHEL-like
systems:

```bash
sudo dnf install -y clang llvm elfutils-devel zlib-devel pkgconf-pkg-config openssl-devel
```

On Debian/Ubuntu-like systems:

```bash
sudo apt-get install -y clang llvm libelf-dev zlib1g-dev pkg-config libssl-dev
```

```bash
cargo build --release
./target/release/actraild start --config docs/examples/01.quick-start/operator.conf
./target/release/actrailctl doctor --config docs/examples/01.quick-start/operator.conf
```

完整端到端流程见 [docs/examples/01.quick-start/README.md](docs/examples/01.quick-start/README.md)。所有公开示例都按目录组织：每个 case 的 README、配置和脚本放在同一个 example 目录下，并包含 Mermaid sequence diagram 说明测试流程。

## Example Cases

| Case | What It Verifies | Entry |
| --- | --- | --- |
| Quick start | Process lifecycle and local network transport through `actraild -> actrailctl -> actrailviewer/actrailweb` | [docs/examples/01.quick-start/README.md](docs/examples/01.quick-start/README.md) |
| LLM HTTP payload and semantics | Real OpenAI-compatible HTTPS LLM request payload capture, HTTPS/1.1 semantic rows, configurable external HTTPS/2, and deterministic local HTTPS/2 frame/DATA capture | [docs/examples/02.llm-http-payload-capture/README.md](docs/examples/02.llm-http-payload-capture/README.md) |
| Extended observation | File path mutations, regular file I/O, MAP_SHARED mmap, IPC, stdio payload, resource metrics, provider labels, timeline/process-tree Web UI | [docs/examples/03.extended-observation-e2e/README.md](docs/examples/03.extended-observation-e2e/README.md) |
| Fanotify enforcement | Trace-scoped permission enforcement with allow/deny file access decisions persisted as Enforcement events | [docs/examples/04.fanotify-enforcement-e2e/README.md](docs/examples/04.fanotify-enforcement-e2e/README.md) |
| HTTP payload unified | Non-TLS HTTP socket plaintext payload capture, HTTP sniff gate, and the same payload/Application viewer surface used by HTTPS | [docs/examples/05.http-payload-unified/README.md](docs/examples/05.http-payload-unified/README.md) |
| Claude Code TLS executable capture | HTTPS payload capture for Claude Code through executable TLS uprobes; default path covers Node/OpenSSL, with optional Bun/BoringSSL patterns | [docs/examples/06.claude-code-tls-capture/README.md](docs/examples/06.claude-code-tls-capture/README.md) |

Recommended reading order is `01 -> 05 -> 02 -> 06 -> 03 -> 04`: start with attach/viewer basics, compare HTTP and HTTPS payload surfaces, then cover executable TLS capture, broad observation, and enforcement.

## Supported Observation Paths

| Area | Current public entry |
| --- | --- |
| Process lifecycle | `actrailviewer processes/events` and `actrailweb` Process Tree |
| Network transport | `actrailviewer network/events` and `actrailweb` Timeline |
| File and IPC events | `actrailviewer events` and `actrailweb` event views |
| TLS plaintext payload | Dynamic OpenSSL and configured executable TLS payload segments through `actrailviewer payloads/payload` and `actrailweb` payload detail |
| HTTP socket plaintext payload | `socket-plaintext-payload` capture for non-TLS HTTP, filtered by the daemon HTTP sniff gate and shown through the same payload viewer commands |
| HTTP/1.x semantic events | Request/response/SSE `Application` rows derived from retained plaintext payloads |
| HTTP/2 frame/DATA facts | Connection preface, frame, and DATA `Application` rows derived from retained TLS plaintext payloads |
| Resource metrics | `actrailviewer events` Resource rows and `actrailweb` Resources tab |
| Stdio payload | `actrailviewer payloads/payload` and `actrailweb` payload detail |
| Provider labels | Rule-set classifier output as durable `Label` events |
| Fanotify enforcement | Trace-scoped allow/deny decisions as durable `Enforcement` events |
| Export | `actrailviewer export-json` |

## Requirements

`actraild` is expected to run with the privileges required by the target Linux/WSL kernel for eBPF tracepoint/uprobe attachment. Fanotify enforcement additionally requires a kernel/environment that supports fanotify permission events. Required capability or environment failures are reported fail-fast instead of silently downgrading collection. Platform and transfer-test preflight checks are documented in [docs/platform-requirements.md](docs/platform-requirements.md). For a colored checklist on a target host, run `python3 docs/preflight/platform_preflight.py --run-smoke --color always`.

AcTrail examples are config-driven and viewer-first: runtime constants live in config files, and verification should use `actrailviewer` or `actrailweb` instead of direct storage inspection.
