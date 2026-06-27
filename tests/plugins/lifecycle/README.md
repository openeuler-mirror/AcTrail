# Plugin Lifecycle Status E2E

This scenario verifies the first real plugin lifecycle control-plane slice.

It starts `actraild` with the existing built-in OTEL JSONL observation consumer, then verifies:

- `actraild plugin list` reports `builtin.otel-jsonl`;
- `actraild plugin status --instance builtin.otel-jsonl` reports the same active runtime instance;
- `actraild plugin unload` deactivates the built-in instance and status then reports not found;
- restarting the daemon restores the config-backed built-in instance.

Run:

```bash
python3 tests/plugins/lifecycle/run_e2e.py
```
