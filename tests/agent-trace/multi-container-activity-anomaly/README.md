# Multi-container activity-anomaly E2E

This test loads two identical `actrail.activity-anomaly` instances into one host daemon and runs two real xiaoO agents in separate Docker containers. A local OpenAI-compatible provider first returns a short warm-up bash call so the first LLM exchange is complete, then asks the agent to run [`long-running-command.sh`](../../../examples/plugins/wit-component/activity-anomaly/long-running-command.sh). The script creates a ready marker, runs for five seconds, and does not finish until the test releases it.

The test verifies, for each trace:

- the long-command alert is persisted just after 500 ms while the provider-issued command is still running, has `status=in_progress`, has no end time, and the trace remains `active`;
- one request-growth alert;
- one response-growth alert;
- one long-command alert with a duration above 500 ms;
- the owning container, trace, process, and Agent action;
- two identical plugin instances do not create duplicate rows;
- command completion and terminal fallback do not duplicate a live alert;
- `last_error=none` for both instances after analysis.

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
