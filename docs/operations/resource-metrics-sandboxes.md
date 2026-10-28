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
added or substituted. Projection of guest observations into a host trace's
resource event series and authenticated guest-to-trace binding are not
implemented.

Workload observation support is negotiated across the guest/gateway/upstream
path. Unsupported peers must not receive the new observation kind; existing
observations continue through the compatible path. Reconnect refreshes capability
state. Guest pressure retains observation code 4; workload cgroup snapshots use
code 5 in both the wire codec and evidence storage. Pressure and workload
collection can run together. The sandbox evidence database uses schema version 3.
Workload observations persist with the other no-plugin-interest evidence. Raw
guest paths identify cgroups within the guest; host trace association requires
separate authorization. Host configs with `[sandbox_evidence] schema_version = 2`
are normalized to 3 before opening storage, independently of guest workload
collection. Newly generated configs use 3.

## Deployment checks

Use a cgroup-v2 guest with a monitored workload in one of the supported layouts.
Compare workload snapshots with the guest's kernel counters, and confirm that
roots in the same cgroup produce one snapshot. Check that observations arrive in
host storage through the guest, gateway and daemon connection. Connection setup
and checks are described in [execution isolation deployment](deployment/execution-isolation.md).
