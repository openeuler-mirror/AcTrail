# Access control

`access_control` defines file, executable and network policies. Operators select a
backend for each of `file.open`, `file.mkdir`, `file.rmdir`, `command.exec` and
`network.connect` under `access_control`. Open supports fanotify/LSM; the other
operations support seccomp/LSM. Observation has its own capture configuration.

## Layout and ownership

- `policy/file`, `policy/command`, `policy/network`: owner rules, grants,
  revisions, validation, effective matching order and dry-run queries.
- The daemon file service owns the single file store and file policy host across
  all backends. It prepares fanotify marks and publishes LSM file entries before
  committing candidate rules; failures retain the existing rules and snapshot.
- `lsm/controller.rs`: owns selected LSM command/network stores and serialized
  publication. Cached file entries derive from successful file-store publications
  and avoid recompiling file rules when other domains change.
- `lsm/compiler.rs`: compile effective rules into a complete immutable LPM map.
- `lsm/publisher.rs`: create, populate, freeze and atomically publish that map
  through one outer map entry. A failed update retains the previous policy.
- `lsm/runtime.rs`, `lsm/audit.rs`: BPF object/link ownership and bounded audit
  draining. `bpf/` contains the independent LSM object and process scope reader.
- The daemon chooses adapters, binds plugin hosts, resolves audit process
  identities and persists enforcement events. The eBPF collector owns process
  launch identity. LSM task storage owns control membership, and the collector
  co-owns the LSM runtime for the daemon lifetime.

Each operation independently selects its execution adapter. The file service
creates a fanotify handle only for fanotify open. File stores select fanotify
coverage validation when open uses fanotify; directory rules need no marks.
No second rule registry or independent PID membership table is introduced.

The kernel must enable BPF LSM and expose BTF, task storage, `bpf_loop`, the
required LSM hooks and directory sequence symbols. Startup checks actual load
and attachment. The implementation has been exercised on Linux 6.6 aarch64.

## Kernel decision path

For a scoped process the hook obtains the current immutable policy map, resolves
its resource, matches a rule and returns zero or `-EACCES`. Explicit policy
precedence is preserved for overlapping recursive paths. Access does not wait
for a userspace request/reply. Audit records carry the matched rule and owner
inline, so later policy updates cannot change their meaning.

Control membership lives in kernel task storage. A root is enrolled from the
published pending-launch binding after checking its start time in procfs ticks.
The `task_alloc` hook inherits control membership before a child runs, including
threads; allocation failure rejects the fork. Task exit releases its storage.
Observation membership-map exhaustion therefore cannot remove control. Kernel
TGID and observer PID remain distinct. Audit identity is registered by the
existing identity service.

LSM attachment is required during startup preflight and when the observation
runtime is loaded. Unsupported kernels, policy compilation failures and attach
failures reject startup/launch. Existing-PID attach is rejected for LSM profiles;
launch provides the registration barrier before workload execution.

## Coverage

| Domain | Hooks | Policy semantics |
| --- | --- | --- |
| File | `file_open`, `path_mkdir`, `path_rmdir` | Exact paths and recursive `/**` scopes; allow/deny |
| Command | `bprm_check_security` | Exact executable paths; allow/deny; no argument predicates |
| Network | `socket_connect` | Numeric IPv4/IPv6 endpoint or address with any port; allow/deny |

File and executable rules keep the declared path for listing and authorization.
Loading or upserting a rule resolves existing symlinks in the daemon's filesystem
view and binds the resulting execution path, including any suffix that will be
created later. A rule declared for A while A points to B continues to govern B
after A is redirected to C. Upserting that rule again binds its current target;
publishing other rules or removing another owner preserves the existing binding.
Bindings identify path names: replacing a regular file at B keeps B governed.

Each path binding has a total work budget, including repeated symlink expansion.
Set `ACTRAIL_POLICY_PATH_MAX_STEPS` in the daemon environment to a positive integer
to override the default of 4096 steps. Root, parent, and name components and
symlink completion steps consume budget before being queued; completed work does
not restore it. This bounds queued steps and synchronous filesystem operations
per binding, including acyclic links that repeatedly expand earlier links.
An invalid setting or exhausted budget rejects the binding. Static rule loading
fails startup; dynamic validation or publication fails locally and retains the
published policy. The same budget applies to grant binding and path dry-runs.

Dynamic authorization requires one grant to cover both the declared scope and
the execution scope within that grant's resolved scope. An exact grant for A
can therefore authorize A's bound target B; a symlink beneath a recursive grant
must remain within that grant's resolved range. File and directory targets may
be absent when configured. Fanotify open additionally needs installable directory
marks; it retains each rule's mark plan across unrelated publications.
Re-enabling a fanotify rule rebuilds its directory mark plan from the stored
execution scope, including descendants created while disabled and removing
directories that no longer exist. Coverage errors reject re-enablement.

Access paths come from the accessing task's filesystem view. `file_open` and
exec use `bpf_d_path`. Directory operations resolve dentries/mounts to task
`fs.root` with bounded traversal and reject inconsistent concurrent path changes
according to `deny_on_error`. Rules apply to path names; existing descriptors,
unlinked aliases and hard links require understanding those path semantics.
Script execution follows the files presented by kernel exec hooks, including
interpreters; command policy does not inspect shell commands or argv.
Network coverage is connection establishment; connectionless sends and existing
connections are outside this hook's coverage.
Static and dynamic network rules accept the same selectors: IPv4 `IP:port` or
`IP:*`, and IPv6 `[IP]:port` or `[IP]:*`. An any-port selector covers all ports
on that address. Static allow/deny rules use `<rule-id> <allow|deny> connect <selector>`.
One owner may publish both an any-port rule and exact endpoint exceptions;
the exact endpoint takes precedence. Static rules share one owner. Duplicate
selectors and overlapping selectors from different owners are rejected.
LSM rejects IPv6 endpoints with a nonzero scope ID (for example
`[fe80::1%2]:443`) before publication because its kernel key cannot represent
interface scope. A rejected update preserves the published policy.

Gray/plugin synchronous decisions and command argument predicates are rejected
at compilation/publication. Kernel decisions use prepublished allow/deny rules.

## Configuration

```toml
[access_control.file.open]
enabled = true
backend = "fanotify"

[access_control.file.mkdir]
enabled = true
backend = "lsm"

[access_control.file.rmdir]
enabled = true
backend = "lsm"

[access_control.command.exec]
enabled = true
backend = "lsm"

[access_control.network.connect]
enabled = true
backend = "lsm"

[access_control.backends.lsm]
policy_max_entries = 4096
path_max_bytes = 252
audit_buffer_bytes = 1048576
audit_batch_max = 1024
audit_default_allow = false
alert_window_ms = 60000
alert_max_groups = 256
deny_on_error = true
deny_signal = "none"
```

The existing `enforcement`, `command_control` and `network_control` sections
retain domain master switches, rules paths, default decisions and audit settings.
Both the domain and operation must be enabled. All operations default to LSM.
File open is enabled by default; directory operations and command/network domains
default to disabled. `init` and `init --mode complete` inherit these defaults;
`profile` disables control domains and removes their capabilities. Complete socket
observation uses seccomp notification to capture full operations independently of
LSM control. LSM prerequisites are checked at startup without a backend fallback.

Capture capabilities must match the selected mechanisms. LSM controls require
`proc-lifecycle` and corresponding `enforcement-file-permission-lsm`,
`enforcement-command-execution-lsm` or `enforcement-network-connect-lsm`.
Fanotify open uses `enforcement-file-permission-fanotify`; directory seccomp uses
`enforcement-file-mkdir-seccomp` and `enforcement-file-rmdir-seccomp`, requiring
`proc-lifecycle` and `seccomp_notify.enabled=true`. Mixed files may request both
fanotify and file LSM capabilities. Directory filters are installed from config,
so rules may be published after launch. Pure BPF observation selects
`capture_backend = "bpf-copy"`; seccomp notification remains independent.

`access_control.backend`, `enforcement.backend` and `enforcement.seccomp_syscalls`
are rejected. Configure each operation explicitly. Cross-backend publication
ensures the new policy after success; it does not promise simultaneous switching
of all mechanisms or atomic updates spanning several policy hosts.

LSM runtime parameters are validated when at least one enabled operation selects
LSM. Inactive backend settings do not block other backends or observation-only
profiles; configuration syntax and capability consistency are still validated.

The policy capacity includes five domain defaults and compiled recursive entries.
It must be at least five. The path limit includes the NUL terminator and is
configurable from 2 to 252 bytes (kernel LPM data limit minus domain selector).
Long rules reject publication; unresolved/long runtime paths use `deny_on_error`.
The audit ring size is a power of two, at least 65536 bytes; batch size is positive.
Default allows are quiet unless `audit_default_allow` is enabled. Matching rules
follow each domain's audit setting. Resource errors produce diagnostic audits.
Ring overflow increments kernel loss counters and emits a warning when drained;
access decisions continue without waiting for audit delivery.

Explicit file and command rule denials generate boundary alerts independently
of audit retention. The daemon groups received denials by trace, operation,
policy owner, rule ID and revision. Each window sends the first denial immediately
and a summary of additional denials when the window or trace ends. Shutdown also
flushes pending summaries. `aggregation.received_denials` is a delta: summing
alerts gives the number of received denials submitted to alert ingress, not an
exact kernel hit count. Ring loss, aggregation capacity and alert delivery
failures can reduce delivered counts. Timestamps cover each delta; path/executable
and process ID describe one sample. LSM file alerts omit `matched_rule_path`
because the kernel only supplies the accessed resource.

`alert_window_ms` and `alert_max_groups` must be positive. Defaults are 60 seconds
and 256 new groups per window, bounding output to at most two alerts per admitted
group per window. Trace completion does not restore admission credits within
that window. Capacity exhaustion suppresses new groups and reports their received
denial count once at window end or shutdown; existing groups keep counting.
Queue admission failure retains the first denial for the summary attempt, without
retrying on every event. Alert delivery failures remain local to observation.

`deny_signal` accepts `none`, `stop` or `kill`. A signal supplements a denied
access. `stop` requests SIGSTOP for the calling process; SIGCONT resumes execution
with the access still denied. It does not retry the syscall or atomically freeze
the process tree. Approval/resume orchestration is a separate future capability.

LSM startup loads the union of configured observation profiles and retains that
runtime until daemon shutdown. An idle observation registry cannot unload live
control membership. Requests exceeding the loaded observation capabilities fail
explicitly. Daemon shutdown releases links and maps. This is session-scoped
enforcement; it does not promise continued confinement after daemon termination.

## Validation

`tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py` runs a real Xiaoo from
refreshed defaults across all-LSM, fanotify/seccomp and mixed configurations. It
verifies OS effects, selected backend audits, first exec, descendants, bpf-copy
payloads and actual fanotify/seccomp resources. It requires a BPF LSM kernel.
To exercise real 24-byte IPv6 connections with explicit allow/deny rules under
both `deny_on_error` settings, run:

```sh
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm LSM_ACCESS_CONTROL_XIAOO_E2E_IPV6=1 \
  python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

Set `LSM_ACCESS_CONTROL_XIAOO_E2E_ALERTS=1` instead of `IPV6=1` to verify
repeated real file/command denials, first and periodic alerts, trace-end summaries,
capacity suppression and alerts with audit retention disabled.

No throughput or latency improvement should be inferred solely from the backend
selection: compare equivalent rules, observation and audit settings on the target
workload. Policy publication is a control-plane cost; access decisions and audit
consumption should be measured separately.
