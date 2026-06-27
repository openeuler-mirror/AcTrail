# Persistent Plugin Load E2E

This scenario verifies explicit persistent plugin lifecycle behavior:

```bash
actraild plugin load --manifest ... --plugin-config ... --instance persistent.otel-jsonl --persist
actraild plugin unload --instance persistent.otel-jsonl --persist
```

Persistence stores only AcTrail-owned instance metadata and file references. The plugin business config remains in the plugin config file.

After restart, the restored plugin must report nonzero `observed_records` in `actraild plugin status`, proving the persisted instance itself consumed the workload.

Plain `actraild plugin unload --instance persistent.otel-jsonl` deactivates only the current runtime instance; the registry record remains and the next daemon start restores the plugin. `unload --persist` removes the registry record.

Run:

```bash
python3 tests/plugins/persistent-load/run_e2e.py
```
