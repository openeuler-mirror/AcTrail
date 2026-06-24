# Container Agent Minimal Operator Config

This example is the recommended host `actraild.conf` baseline when a workload container runs only `actrailctl launch -- <agent>` against a host daemon.

## What it changes

- Uses production socket paths under `/run/actrail` and SQLite under `/var/lib/actrail`.
- Keeps TLS-sync enabled for LLM/plaintext capture.
- Disables launch-time process seccomp (`process_seccomp_enabled = false`).
- Sets `payload_tls_binary_path` so launch can fall back to a fixed probe plan when dynamic binary probing fails inside a restricted container.

Replace `/usr/local/bin/agent` with the actual agent binary path inside the container.

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
