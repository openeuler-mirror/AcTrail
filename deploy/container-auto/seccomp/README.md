# AcTrail Docker seccomp-notify profile

`actrail-notify.json` is generated from Moby's versioned
[`seccomp/v0.2.1` default profile](https://github.com/moby/profiles/blob/seccomp/v0.2.1/seccomp/default.json).
The generator pins the upstream SHA-256 and makes exactly one policy change:

- remove `pidfd_getfd` from the rule gated by `CAP_SYS_PTRACE`;
- allow `pidfd_getfd` unconditionally.

AcTrail uses `pidfd_getfd` only to copy the seccomp listener from the child it
just forked before allowing that child to exec. This keeps Docker's default
deny-by-default policy and avoids granting the workload `CAP_SYS_PTRACE`,
`CAP_SYS_ADMIN`, `CAP_BPF`, or `CAP_PERFMON`.

This JSON profile does not contain or imply `seccomp=unconfined`.
`seccomp=unconfined` is a Docker runtime option that skips Docker's outer
seccomp profile entirely. It remains usable for trusted testing or compatibility
diagnosis, and AcTrail can still install its own seccomp-notify filter inside
the container, but the Docker syscall allowlist is no longer present.

Regenerate and verify:

```bash
python3 deploy/container-auto/seccomp/generate-notify-profile.py
python3 -m json.tool deploy/container-auto/seccomp/actrail-notify.json >/dev/null
```

Run a workload with seccomp-notify available:

```bash
docker run \
  --security-opt seccomp="$(pwd)/deploy/container-auto/seccomp/actrail-notify.json" \
  ...
```
