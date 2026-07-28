# Multi-container activity-anomaly E2E

This test loads `actrail.activity-anomaly` into one host daemon and runs two real xiaoO agents in separate Docker containers. A local OpenAI-compatible provider returns a bash tool call that executes `sleep 2`.

The test verifies, for each trace:

- all alerts are persisted while the real xiaoO process is still running a provider-issued hold command and the trace remains `active`;
- one request-growth alert;
- one response-growth alert;
- one long-command alert with a duration above 500 ms;
- the owning container, trace, process, and Agent action;
- terminal fallback does not duplicate a live alert;
- `last_error=none` after analysis.

## Run

Set the repository path and build the release artifacts:

```bash
export ACTRAIL_REPO="<path-to-AcTrail>"
cd "$ACTRAIL_REPO"

cargo fmt --all
cargo build --release
cargo build --release --target wasm32-wasip2 \
  --manifest-path examples/plugins/wit-component/activity-anomaly/Cargo.toml
```

Run the E2E test:

```bash
sudo python3 \
  tests/agent-trace/multi-container-activity-anomaly/run_e2e.py
```

For Docker or runc environments that reject the outer seccomp profile before container startup, use:

```bash
sudo python3 \
  tests/agent-trace/multi-container-activity-anomaly/run_e2e.py \
  --seccomp-profile unconfined
```

This compatibility option only disables the Docker outer filter. The test still requires AcTrail host eBPF and seccomp notify.

The test uses an isolated system temporary directory and removes its containers and runtime files on exit. Use `--keep-runtime` only for failure diagnosis.
