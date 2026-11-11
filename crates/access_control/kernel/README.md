# LSM grey decision module

`actrail_lsm_grey` lets a sleepable BPF LSM program wait for an external authorization decision. AcTrail keeps allow and deny rules in the kernel policy map. A grey rule supplies a control plugin, a deadline, a concurrency limit, and a fallback decision.

For a grey access, the module copies the request into a bounded queue and sleeps the calling thread. The daemon reads `/dev/actrail_lsm_grey`, invokes the selected WASM plugin, and writes its verdict. The plugin can call an independent local HTTP security service. The module wakes the original thread, and the LSM hook returns the decision to the syscall.

## Build and load

The running kernel must enable BPF LSM and permit custom modules. Install matching kernel headers, the kernel compiler, `pahole`, and the headers package's `resolve_btfids` tool. Build against the running kernel's BTF:

```sh
bash crates/access_control/kernel/build.sh
sudo insmod crates/access_control/kernel/actrail_lsm_grey.ko \
  max_pending=64 \
  max_wait_ms=30000
```

`ACTRAIL_KERNEL_BUILD` overrides the headers directory, which defaults to `/lib/modules/$(uname -r)/build`. `ACTRAIL_KERNEL_BTF` overrides the BTF file, which defaults to `/sys/kernel/btf/vmlinux`. Use files from the target kernel when building for another machine.

Enable the channel in the daemon configuration:

```toml
[access_control.backends.lsm.grey]
mode = "auto"
device_path = "/dev/actrail_lsm_grey"
workers = 4
```

`scripts/install-release.sh` attempts to build and install the module for the running kernel when build prerequisites are available. It runs `depmod` after installation and leaves loading to the daemon. Missing prerequisites or build failure do not stop installation.

The default `auto` mode attempts `modprobe actrail_lsm_grey` only when effective access-control routing selects LSM. Module or device failures select ordinary LSM with each grey rule's fallback. If the grey BPF object cannot load, the runtime retries the ordinary object. Policy errors remain fatal. Startup logs report dynamic selection or the fallback reason.

Use `required` to require dynamic grey and fail startup when unavailable. Use `disabled` to skip module loading and workers. Other control backends never trigger loading. The device allows one daemon session and requires root access. Dynamic programs require the module's `actrail_lsm_decide` and `actrail_lsm_exec_decide` kfuncs.

## Limits and fallback

| Setting | Default | Meaning |
| --- | --- | --- |
| `max_pending` | 64 | Maximum queued and executing requests across all rules |
| `max_wait_ms` | 30000 | Maximum wait per request, also bounded by the rule timeout |
| `argv_max_count` | 16 | Maximum captured exec arguments, configurable from 1 to 16 |
| `argv_arg_max_bytes` | 256 | Bytes per argument including NUL, configurable from 2 to 256 |

For command grey requests, `actrail_lsm_exec_decide` receives the current `linux_binprm` from the sleepable exec hook. It reads all `argc` strings from `bprm->p` in the new image's `bprm->mm`, including strings crossing page boundaries. This is the kernel's argument copy for native ELF execution. Concurrent changes to the caller's original argv do not change this copy. Page references and the mmap read lock are released before queueing or waiting. The resolved executable and actor identity come from the LSM event; no separate PID lookup or pending table is needed.

An unreadable argument, excessive argument count or length, or invalid syscall context returns `AC_GREY_CONTEXT_INVALID`. The BPF program denies this access even with allow fallback. Successfully captured LSM contexts contain complete argv. The requested path comes from the kernel-owned `bprm->filename`; execveat may use a synthesized `/dev/fd/...` name. Its buffer is 256 bytes including NUL, and overflow also denies. Syscall type, dirfd and flags come from the current thread's syscall registers. The device message remains 5368 bytes. Load matching module and daemon builds; stop the daemon and unload the module before replacing it.

The module also enforces each rule's concurrency limit. It groups pending requests by plugin target, rule owner, and rule ID. It keeps the rule snapshot from the intercepted access until completion. Policy updates affect subsequent accesses.

Queue saturation, timeout, interruption, or daemon disconnection uses the rule's fallback. Closing the device wakes pending callers. Replies for completed or expired requests are rejected. A missing or unloaded plugin produces deny. Plugin execution failures use the rule's fallback.

The grey BPF variant uses task storage to keep the request and matching state valid while the thread sleeps. Ordinary allow and deny accesses do not enter the device queue. Disabling grey uses the ordinary BPF variant and resolves grey rules through their configured fallback.

## Hook support

The sleepable variant uses these hooks: `file_open`, `mmap_file`, `path_mkdir`, `path_rmdir`, `path_unlink`, `path_chmod`, `path_chown`, `path_truncate`, `path_rename`, `path_link`, `bprm_check_security`, `socket_connect`, `socket_sendmsg`, `socket_bind`, and `socket_listen`.

`file_permission`, `file_mprotect`, and `file_truncate` use the rule's fallback immediately. For example, a read rule may invoke the service during `file_open` and then use fallback during the later `file_permission` check. Choose operation rules and fallback with both checks in mind. `task_alloc` only propagates the controlled scope.

Run the daemon and security service outside controlled traces. The service should use its own rules and template files. A sleepable hook can still hold filesystem locks. Keep the service independent of the blocked caller and its resources. The bounded deadline terminates a stalled decision.

## Integration test

See [the real Xiaoo scenario](../../../tests/v2/regression/lsm_grey_xiaoo/README.md) for static decisions, repeated external decisions on one path, read fallback, and service failure cases.

Stop the daemon and detach its BPF programs before unloading:

```sh
sudo rmmod actrail_lsm_grey
```
