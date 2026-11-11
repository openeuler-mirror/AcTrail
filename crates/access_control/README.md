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

Each operation's immutable policy default records whether resource inspection
is required. Adding or removing rules updates this requirement in the same
snapshot.

| Policy for the current operation | Scratch and resource inspection | Behavior |
| --- | --- | --- |
| No rules, default allow, no audit | Skipped | Allow after scope and policy lookup |
| No rules, default deny | Retained | Apply the default decision and configured audit |
| No rules, default allow with audit | Retained | Allow and record the resource |
| Resource rules exist | Retained | Match the resource and apply the selected decision |

Combined file operations check all relevant permissions in the same snapshot.
They resolve the path once when at least one permission requires it.

Preparation is lazy within each hook invocation. Scope and policy lookup run
once; rename and link reuse that context for both names. A default lookup uses
a prebuilt read-only key, so scratch lookup or allocation occurs only when a
domain requires a resource. The context expires when the hook returns. Rename
still checks source, target and any overwritten object's delete permission;
link still checks both names. Policy updates take effect on the next hook.

The hook reads the selected rule directly from the frozen policy map. The
context holds a rule reference for the current BPF invocation. Kernel RCU
protects the map lifetime, including tasks-trace RCU during sleepable grey
decisions. Rule identifiers and revision are copied only when creating an audit
record or grey request. Grey timeout and concurrency values are read when
entering the grey branch.

Resource scratch is reused without clearing its 252-byte capacity on entry.
The hook tracks the valid length of each resolved path or endpoint. Failed
resolution leaves no valid resource. Audit and grey records initialize their
fixed-size resource field at emission and copy only the valid bytes.

Measure retained file reads, writes and socket sends with a real Xiaoo process
and freshly generated configuration:

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/retained/run.py run \
  --order off empty lsm lsm empty off \
  --count 1000 \
  --rounds 5 \
  --warmup 100 \
  --timeout 120
```

| Mode | LSM control | Benchmark rule files |
| --- | --- | --- |
| `off` | Disabled | Rules are not enforced |
| `empty` | Enabled | Empty |
| `lsm` | Enabled | Explicit allow rules for file open and network connect |

Both enabled modes use default allow with audit disabled. The timed read, write
and send operations have no explicit rules in either mode.

For a scoped process the hook obtains the current immutable policy map, resolves
its resource, matches a rule and returns zero or `-EACCES`. Explicit policy
precedence is preserved for overlapping recursive paths. Static allow and deny
complete in the kernel. Grey can wait for a userspace decision. Audit records
carry the matched rule and owner
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
| File | `file_open`, `file_permission`, `mmap_file`, `file_mprotect`, `path_mkdir`, `path_rmdir`, `path_unlink`, `path_chmod`, `path_chown`, `path_truncate`, `file_truncate`, `path_rename`, `path_link` | Exact paths and recursive `/**` scopes, with operation-specific allow/deny |
| Command | `bprm_check_security` | Exact executable paths; allow/deny; no argument predicates |
| Network | `socket_connect`, `socket_sendmsg`, `socket_bind`, `socket_listen` | Operation, protocol and numeric IPv4/IPv6 endpoint |

File operations include `open`, `read`, `write`, `delete`, `attr`, `rename`, `link`, `mkdir`, `rmdir` and `any`.
`open` gates a new file open. Opening for reading or writing also checks the
corresponding operation. Reads and writes through an existing descriptor check
`read` and `write` independently. Truncation through a path, a retained descriptor
or `O_TRUNC` requires `write`, enforced by `path_truncate` and `file_truncate`.
File-backed mappings with access require `read`.
Shared writable mappings also require `write`. Private copy-on-write mappings
require `read` because they do not modify the file. `mprotect` checks permission
increases and permits reductions. Policy changes govern subsequent checks and
do not revoke memory access already granted to a mapping.

Read/write, deletion, attributes, rename and hard-link enforcement use the LSM backend selected by `access_control.file.open`.
Explicit read/write/delete/attr/rename/link rules are rejected when that backend is unavailable. `any`
expands to the operations supported by the selected backend. The file domain's
default decision applies separately to each operation. Builtin open allows only
cover the open gate.

`delete` checks unlink of a file directory entry. `attr` checks mode, owner and
group changes through chmod/chown, including their descriptor variants. These
operations use independent rules, so a writable file can remain protected from
removal and attribute changes. Extended attributes are outside the attr hooks.

`rename` checks both source and destination paths. Replacing an existing target
also requires `delete` on that target. `RENAME_EXCHANGE` checks `rename` on both
names because both objects remain present. Renaming between two names of the same
inode requires rename permission and preserves both names. `link` checks both the existing source
and the destination name. These checks govern path names, including directory-FD
relative operations.

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
Network rules select `operation=connect/send/bind/listen`, `protocol=any/tcp/udp`
and one `endpoint`. Connect/send select the remote endpoint. Bind/listen select
the local endpoint. Bind checks the requested port, including zero for automatic
allocation. Listen checks the socket's current local port, including zero before
an unbound socket receives an automatic port. TCP sends use the
connected peer, except TCP Fast Open before connection establishment. UDP sends
use the supplied destination or the connected peer.

Selectors combine a numeric IP or CIDR with an inclusive port scope: `443`,
`8000-8100`, or `*`. Examples are `192.0.2.0/24:443`, `192.0.2.1:8000-8100`
and `[2001:db8::/48]:*`. CIDR host bits are normalized to the network address.
The longest IP prefix wins, then the narrowest port range, then an explicit
protocol over `any`. Equal specificity uses owner and rule ID order. Different
operations match independently. The same owner can publish narrower exceptions.

Overlapping rules from different owners are rejected when their protocols and
operations intersect. Grants constrain decision, operation, protocol and the
complete address/port scope. A TCP-only grant cannot authorize an any-protocol
rule. A rule must fit entirely within one grant.

Network indexes are rebuilt on publication. Userspace queries existing IP
prefixes and disjoint port segments. LSM compiles a port interval into at most
30 prefixes and queries only active prefix lengths, at most 17 per protocol.
The snapshot capacity includes the expanded entries. A candidate exceeding
`policy_max_entries` is rejected before activation.

Static allow/deny rules use `<id> <allow|deny> <operation> <endpoint>` with an
optional `protocol <any|tcp|udp>` suffix. Protocol defaults to any. Dynamic JSON
rules default to connect/any. The seccomp backend accepts only connect/any and
rejects other rules before publication. LSM rejects nonzero IPv6 scope IDs
because its kernel key does not include an interface scope.

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

The policy capacity includes fourteen domain defaults and compiled recursive entries.
The compiled snapshot must fit within the configured capacity. The path limit includes the NUL terminator and is
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
