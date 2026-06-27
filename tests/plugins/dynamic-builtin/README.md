# Dynamic Built-In Plugin Load E2E

This scenario verifies the first real `actraild plugin load/unload` path.

It starts `actraild` without startup live export routes, then loads the built-in `otel-jsonl` observation consumer through:

```bash
actraild plugin load --manifest ... --plugin-config ... --instance dynamic.otel-jsonl
```

The plugin config is plugin-owned and is not embedded in the daemon operator config.

Run:

```bash
python3 tests/plugins/dynamic-builtin/run_e2e.py
```
