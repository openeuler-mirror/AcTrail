# Container Agent Minimal Operator Config

This example is the recommended host `actraild.conf` baseline when a workload container runs only `actrailctl launch -- <agent>` against a host daemon.

## What it changes

- Uses production socket paths under `/run/actrail` and SQLite under `/var/lib/actrail`.
- Keeps TLS-sync enabled for LLM/plaintext capture.
- Disables launch-time process seccomp (`[process_seccomp] enabled = false`).
- Keeps `[payload.tls] binary_path = "disabled"` because the tls-sync backend builds the probe plan dynamically at launch; a fixed path is rejected with `tls-sync auto plan requires payload.tls.binary_path=disabled`. (The `binary_path` option is only a fallback for the non-sync executable-source path.)

For the full degradation chain — how a container with the default seccomp profile and no `CAP_BPF` still captures TLS plaintext via the tls-sync inline-hook path — see [container-agent-restricted](../container-agent-restricted/README.md#how-the-degraded-path-captures-tls-plaintext-without-seccomp).

## Recommended container workflow

```bash
# Inside the workload container (host sockets mounted at /run/actrail):
actrailctl --config /etc/actrail/actraild.conf probe
actrailctl --config /etc/actrail/actraild.conf launch -- /usr/local/bin/agent ...
```

`launch` defaults to `--seccomp-mode auto`. When Docker still applies a default seccomp profile and pidfd launch is unavailable, ctl degrades to tls-sync-only launch instead of failing immediately.

For full socket/process seccomp capture from the container, keep `--security-opt seccomp=unconfined` on the workload container.

## Install on host

```bash
sudo install -d /etc/actrail /var/lib/actrail /run/actrail /var/log/actrail
sudo install -m 0644 docs/examples/container-agent-minimal/operator.conf /etc/actrail/actraild.conf
./target/release/actraild --config /etc/actrail/actraild.conf start
```

Mount `/etc/actrail` read-only and `/run/actrail` read-write into the workload container.
