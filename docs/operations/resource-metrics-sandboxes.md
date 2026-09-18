# Sandbox workload memory accounting

The guest agent can collect charged memory for each recognized workload cgroup
inside the sandbox. Enable this in the `actrail-sb` daemon configuration:

```toml
[sampler]
poll_interval_ms = 1000
workload_cgroups = true
```

The default is false. Existing `[collector].root_process_names` identifies the
monitored roots. The guest needs cgroup v2 and readable procfs/cgroup counters.
Accepted container boundaries are `/default/<64-lowercase-hex-id>` and
`/k8s.io/<64-lowercase-hex-id>`; descendants resolve to that container boundary.
Roots sharing a boundary produce one workload snapshot with a monitored-root
count. Identity includes guest boot, mount, directory identity and canonical path;
PID start times are checked around resolution. An unreadable workload is skipped
locally rather than represented by zero counters.
Identity metadata and counters come from the same open directory descriptor;
pathname replacement cannot combine different cgroups. Discovery/resolution errors
are isolated per root, including processes disappearing during discovery.

Workload snapshots carry charged current/optional peak memory and available CPU,
I/O, PID and memory-event counters. Guest-wide resource snapshots remain separate.
Guest memory and host VMM memory measure different boundaries and must not be
added or substituted. This PR does not project guest observations into a host
trace's resource event series or implement authenticated guest-to-trace binding.

Workload observation support is negotiated across the guest/gateway/upstream
path. Unsupported peers must not receive the new observation kind; existing
observations continue through the compatible path. Reconnect refreshes capability
state. Guest pressure retains observation code 4; workload cgroup snapshots use
code 5 in both the wire codec and evidence storage. Pressure and workload
collection can run together. The sandbox evidence database advances independently from version 2 to 3;
workload observations persist with the other no-plugin-interest evidence. Raw
guest paths are local identity inputs, not wire-authorized host trace selectors.
Existing generated host configs with `[sandbox_evidence] schema_version = 2` are
accepted and normalized to 3 before opening storage. This upgrade does not require
enabling guest workload collection; newly generated configs use 3.

## Verification

```bash
cargo test -p sandbox_observation -p sandbox_linux_collector -p sandbox_vsock_contract -p sandbox_upstream_contract
cargo test -p sandbox_agent_runtime -p vsock_gateway_runtime -p sandbox_evidence_sqlite -p daemon -p sb
cargo test -p sandbox_evidence_sqlite --test workload_pipeline
```

The pipeline integration test sends two distinct workloads through the production
wire codec, async SQLite writer and store reopen, checking identity, source,
sequence, route generation, absent counters and full-width values. Other tests
exercise parsing, negotiation, payload rejection and schema migration.

For live acceptance, build and run the following test inside a cgroup-v2 guest
with an idle monitored workload and a supported layout (replace `sleep` with its
process comm). It compares guest counters with direct kernel readings, checks
deduplication, and exercises the same wire/storage path:

```bash
ACTRAIL_TEST_GUEST_ROOT_COMM=sleep cargo test -p sandbox_evidence_sqlite --test workload_pipeline live_guest_counters -- --ignored --nocapture
```

The live test is ignored by default and fails if explicitly run without its
prerequisites. The pipeline test does not establish live vsock/mTLS delivery;
deployment acceptance should also run the existing sandbox transport regressions
on a VM-capable host. See the [validation record](../designs/resource-metrics-validation.md).
