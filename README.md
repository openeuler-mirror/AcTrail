# AcTrail

[English](README.md) | [中文](README.zh-CN.md)

> **Action Trail, Actual Trail.** Verify what an agent does, not just what it says.

## What is AcTrail

AcTrail is an observability and governance foundation for AI agents running on Linux and WSL. It records what an agent process tree actually does, reconstructs higher-level actions from system and protocol evidence, and gives security, development, and operations teams a traceable basis for investigation and control.

A single trace can connect process launches, file and IPC activity, network connections, TLS plaintext, HTTP semantics, LLM requests and responses, tool calls, resource signals, diagnostics, and policy decisions.

## Why you will need AcTrail

![Why self-reported trace is not enough](./images/figure1-agent-log-gap-zh-cn.drawio.svg)

An agent log describes what the agent chose to report. It does not reliably cover scripts, subprocesses, modified files, network traffic, or incomplete and degraded execution. AcTrail provides independent system-level evidence for questions such as:

- What process tree ran, and which commands did it spawn?
- Which files, sockets, pipes, and network endpoints did it touch?
- What did it send to an LLM provider, and what came back?
- Which payload, HTTP, or process event proves a higher-level action?
- Which observations were complete, partial, blocked, or degraded?

![AcTrail evidence-to-action trail](./images/actrail-readme__evidence-to-action__candidate.drawio.svg)

## Core Features

- **System-level agent observation**: observe Linux/WSL process trees independently of agent-generated logs, including commands, files, mmap, IPC, stdio, sockets, network activity, and resource signals.
- **Encrypted-traffic evidence**: capture authorized TLS plaintext and connect payload metadata to socket and process identity.
- **Protocol and semantic reconstruction**: rebuild HTTP/1, HTTP/2, SSE, LLM request/response, reasoning, tool-call, and usage semantics from lower-level evidence.
- **Evidence-linked action trails**: correlate semantic actions with process ancestry, identities, payloads, diagnostics, completeness, and degradation state.
- **Governance and alerting**: apply fanotify and seccomp decisions, load policy and analysis plugins, persist alerts, and forward selected alerts without coupling downstream failures to collection.
- **Local and external analysis**: inspect traces through the Web UI or CLI, export JSON/OpenTelemetry data, and optionally upload terminal traces to a cluster service.
- **Multiple deployment boundaries**: run on a Linux/WSL host, observe Docker workloads from a host daemon, or use execution isolation when the workload needs a separate guest trust boundary.

## Deployment View

AcTrail has two distinct deployment families. A regular Host/VM deployment runs the complete AcTrail runtime beside the Agent. An execution-isolation deployment separates the Agent's Brain from a remote Hand sandbox and uses the lightweight `actrail-sb` path inside that sandbox.

### Agent and actraild in one runtime boundary

A physical Linux Host and a regular Linux VM use the same topology: `actraild`, storage, and the Agent run inside the same operating-system boundary. If the Agent runs in Docker, `actraild` remains on the Docker Host; the workload uses the configured local AcTrail control and payload channels, while Host collectors observe its system activity. A full VM must run its own `actraild` inside the Guest because Host eBPF collection cannot replace Guest-kernel collection.

![AcTrail deployment with the Agent and actraild in one runtime boundary](./images/readme-deployment-local.svg)

### Brain and remote Hand sandbox on different Hosts

For execution isolation, the Brain runs on one Host while the Hand runs inside a sandbox Guest on another Host. The Guest runs one `actrail-sb`, not a full `actraild`. The Hand Host terminates the VMM's VSOCK endpoint with `actrail-vsock-gateway`; the gateway forwards observation frames to `actraild` on the Brain Host. Firecracker is the checked-in default for this path, with StratoVirt and Cloud Hypervisor available as alternative backends.

![AcTrail deployment with an Agent Brain and a remote Hand sandbox](./images/readme-deployment-hand-brain.svg)

The gateway-to-daemon transport is currently plain TCP with AcTrail framing. A cross-Host deployment must protect this link with a trusted private network or an external secure tunnel; it must not be exposed directly to an untrusted network. See [Choose a deployment mode](docs/operations/deployment/choose-a-mode.md), [default deployment architecture](docs/architecture/deployment/default.md), and [execution-isolation deployment](deploy/execution-isolation/README.md).

## Prerequisites

- Linux or WSL. `x86_64` is verified; ARM64 has code support but is not yet verified on the target distributions.
- Root or equivalent kernel capabilities for live collection.
- Kernel BTF, writable tracefs controls, and permission to attach perf tracepoints and uprobes.
- Rust `1.90+` with `rustup` and the `wasm32-wasip2` target available to the installer.
- Clang/LLVM, libelf, zlib, pkg-config, OpenSSL development packages, and a musl toolchain.
- Node.js `18+` and npm for the Web frontend.

The dependency installer supports `dnf` and `apt-get`. For kernel and feature-specific requirements, read [Platform support](docs/reference/platform-support.md).

## Installation

### For an AI Agent

Give the following instruction to an AI coding agent working in an AcTrail source checkout:

```text
Install AcTrail from this repository on the current authorized Linux/WSL host.
First read README.md, docs/reference/platform-support.md, and
scripts/install-release.sh. Check the build prerequisites, then run the release
installer with /usr/local/bin as the destination. Do not weaken host security
settings or overwrite an existing AcTrail configuration. Report missing kernel
capabilities or dependencies instead of hiding them. Finally verify that
actraild, actrailctl, actrailviewer, and actrailweb are available on PATH.
```

The agent should use the repository installer rather than reconstructing build and copy steps manually.

### Manual Installation

#### From Source

Install or check build dependencies from the repository root:

```bash
./scripts/install-build-deps.sh --install
```

Build release artifacts and install the binaries, TLS runtimes, and official plugin packages:

```bash
./scripts/install-release.sh /usr/local/bin
```

The installer uses `sudo` only when the destination requires elevated permissions. By default, plugin packages are installed under `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}` and remain disabled until explicitly loaded.

#### RPM

Download the package matching the target distribution and architecture from the [latest release page](https://gitcode.com/openeuler/AcTrail/releases/latest):

```text
AcTrail-<VERSION>-<RELEASE>.<DISTRO>.<ARCH>.rpm
```

Install or upgrade it with:

```bash
sudo rpm -Uvh AcTrail-<VERSION>-<RELEASE>.<DISTRO>.<ARCH>.rpm
```

## Quick Run

The default configuration performs broad collection and can persist sensitive plaintext, including prompts, API keys, authorization headers, and model responses. Use the first run only on an authorized, disposable development host or workload.

After installation, initialize the configuration, start the daemon, launch a traced command, and open the local Web UI:

```bash
sudo actraild init
sudo actraild start
sudo actrailctl launch --name quickstart -- \
  bash -lc 'echo ACTRAIL_QUICKSTART_OK; id >/dev/null; ls /etc/hosts >/dev/null'
sudo actrailweb
```

Open `http://127.0.0.1:18080`, select the `quickstart` trace, and inspect its process tree, actions, evidence, and diagnostics. `actrailweb` runs in the foreground; press `Ctrl-C` to stop it, then stop the daemon:

```bash
sudo actraild stop
```

For CLI verification and troubleshooting, follow the [complete Quickstart](docs/getting-started/quickstart.md).

## More Documentation

| Goal | Documentation |
| --- | --- |
| Understand supported security questions and evidence | [Capability overview](docs/concepts/capabilities.md) |
| Check collection coverage | [Collection and observation checklist](docs/concepts/collection-observation-checklist.md) |
| Review trust, data, and privilege boundaries | [Security model](docs/concepts/security-model.md) |
| Configure collection and retention | [Collection configuration](docs/reference/configuration/collection.md) |
| Operate and troubleshoot AcTrail | [Operations guide](docs/operations/README.md) |
| Understand the implementation | [Architecture](docs/architecture/README.md) |
| Browse all documentation | [Documentation index](docs/README.md) |

## License

AcTrail is licensed under the [Mulan Permissive Software License, Version 2](LICENSE).

The eBPF C programs contain Linux kernel verifier license-section strings for BPF loading and helper compatibility. Those strings do not replace the repository-level license.
