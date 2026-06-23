# AcTrail Preflight Tools

This directory contains transfer-test preflight tooling. Run commands from the repository root.

## Read-Only Scan

```bash
python3 docs/preflight/platform_preflight.py --color always
```

Use this mode to check the host, release artifacts, toolchain, shared OpenSSL, and optional agent executables such as `claude` and `opencode`.

By default, the scan looks for AcTrail release artifacts under `target/release` and then on `PATH` when an executable is missing there. Set `ACTRAIL_BIN_DIR` or pass `--bin-dir` to point at either a release artifact directory or one known release artifact path, for example:

```bash
python3 docs/preflight/platform_preflight.py --bin-dir /opt/actrail/bin
python3 docs/preflight/platform_preflight.py --bin-dir /opt/actrail/bin/actraild
```

When a direct binary path such as `.../actraild` is supplied, preflight checks sibling release artifacts from that same directory.

The TLS sync smoke also requires `libactrail_tls_payload_probe_sync.so` next to `actrailctl`, unless `TLS_PAYLOAD_SYNC_LIBRARY` points at a readable runtime library.

When `--run-smoke` executes the eBPF live attach check, preflight runs the resolved `ebpf_probe` path with `ACTRAIL_EBPF_LIBBPF_DEBUG=1`. If the kernel rejects the embedded eBPF object, the failure row includes the actual probe path and a bounded libbpf/verifier log excerpt. Use `--verbose` to print the full captured stdout/stderr.

Status symbols:

- green `✓`: usable.
- red `✗`: failed. A `[required]` failure blocks transfer testing.
- yellow `!`: warning or runtime proof not run.

## Full Local Smoke

```bash
python3 docs/preflight/platform_preflight.py --run-smoke --color always
```

This mode also runs the documented local eBPF live attach, HTTP/2 TLS seccomp payload, and fanotify enforcement checks. It should be the default handoff command for QA on a new host.

The agent executable TLS rows are optional. They only block the corresponding agent-specific example. For example, the Claude Code TLS example requires `claude` plus a complete `tls-probe-point-finder fast --provider auto --source auto` plan for the local Claude runtime.

## Claude Native TLS Profile

Some Claude Code installs are npm wrapper packages whose postinstall step places a native ELF executable under `@anthropic-ai/claude-code/bin/`. To inspect the local runtime without exporting the binary, run the profile script on the target host:

```bash
python3 docs/preflight/claude_native_profile.py \
  --json-output /tmp/actrail-claude-native-profile.json
```

`status=supported` means the host can run the Claude TLS capture example with the reported auto-plan provider. `status=profile_missing` means the executable stayed on the target host, but AcTrail still needs finder support for that runtime before the Claude payload case can pass. The JSON contains the package name/version, native package, arch, GNU build-id, SHA-256, OpenSSL symbol scan, and fast-plan error. needed to add that profile.
