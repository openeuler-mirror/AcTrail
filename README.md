# AcTrail

> AcTrail: Action Trail, Actual Trail
> Don't rely on what agent says, verify what it does

AcTrail is a Linux system-observation toolkit for AI agents. It captures
process, network, file, IPC, payload, HTTP semantic events, and resource samples
from real runtimes, projects raw runtime evidence into trustworthy
framework-independent agent action trails, and exposes the result through
`actrailviewer` and the read-only `actrailweb` UI.

## Components

```text
actraild -> libbpf eBPF collector -> ingest/analyzers -> AcTrail storage -> actrailviewer/actrailweb
```

| Component | Role |
| --- | --- |
| `actraild` | Collection daemon for the eBPF collector, resource sampler, analyzer lifecycle, trace attachment, and storage writes. |
| `actrailctl` | Control-plane CLI for `doctor`, `launch`, `track-add`, `track-remove`, `list-traces`, and `clean`. |
| `actrailviewer` | Read-only CLI viewer for traces, events, network activity, payloads, diagnostics, and JSON export through the configured `storage_path`. |
| `actrailweb` | Read-only Web UI for trace lists, metric summaries, agent-centered action swimlane/tree views, and JSON/payload/detail panels through the configured `storage_path`. |

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

Building from source also compiles and embeds `actrailweb` assets, so `node` and `npm` must be available on `PATH` before `cargo build --release`; Node.js 20 is recommended. The resulting `actrailweb` release binary serves the embedded assets directly and does not require Node.js, npm, or `node_modules` at runtime.

```bash
node --version
npm --version
npm ci --prefix crates/apps/web/frontend
cargo build --release
./target/release/actraild --config docs/examples/08.full-monitor-validation/operator.conf start
./target/release/actrailctl --config docs/examples/08.full-monitor-validation/operator.conf doctor
```

For real agent end-to-end validation, start with
[docs/examples/08.full-monitor-validation/README.md](docs/examples/08.full-monitor-validation/README.md).
For the full examples index, see
[docs/examples/README.md](docs/examples/README.md).

## Supported Observation Paths

| Area | Current public entry |
| --- | --- |
| Process lifecycle | `actrailviewer processes/events` and actrailweb process metrics/details |
| Network transport | `actrailviewer network/events` and actrailweb event evidence details when linked to semantic actions |
| File and IPC events | `actrailviewer events` and actrailweb action/evidence detail panel |
| TLS plaintext payload | Dynamic OpenSSL and configured executable TLS payload segments through `actrailviewer payloads/payload` and `actrailweb` payload detail |
| HTTP socket plaintext payload | `socket-plaintext-payload` capture for non-TLS HTTP, filtered by the daemon HTTP sniff gate and shown through the same payload viewer commands |
| HTTP/1.x semantic events | Request/response/SSE `Application` rows derived from retained plaintext payloads |
| HTTP/2 frame/DATA facts | Connection preface, frame, and DATA `Application` rows derived from retained TLS plaintext payloads |
| Resource metrics | `actrailviewer events` Resource rows and actrailweb trace metric/detail surfaces |
| Stdio payload | `actrailviewer payloads/payload` and `actrailweb` payload detail |
| Provider labels | Rule-set classifier output as durable `Label` events |
| Fanotify enforcement | Trace-scoped allow/deny decisions as durable `Enforcement` events |
| Export | `actrailviewer export-json` |

## Requirements

`actraild` is expected to run with the privileges required by the target Linux/WSL kernel for eBPF tracepoint/uprobe attachment. Fanotify enforcement additionally requires a kernel/environment that supports fanotify permission events. Required capability or environment failures are reported fail-fast instead of silently downgrading collection. Platform and transfer-test preflight checks are documented in [docs/platform-requirements.md](docs/platform-requirements.md). For a colored checklist on a target host, run `python3 docs/preflight/platform_preflight.py --run-smoke --color always`.

AcTrail examples are config-driven and viewer-first: runtime constants live in config files, and verification should use `actrailviewer` or `actrailweb` instead of direct storage inspection.

## License

AcTrail is licensed under the Mulan Permissive Software License, Version 2. See [LICENSE](LICENSE).

The eBPF C programs include Linux kernel verifier license-section strings such as `char LICENSE[] SEC("license") = "GPL";`. Those strings are kept for BPF loading/helper compatibility and do not replace the repository-level license declaration in `LICENSE`.
