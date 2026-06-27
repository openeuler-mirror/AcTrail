# WASM Observation Plugin E2E

This scenario verifies the first customer-authored WASM observation consumer path.

It starts `actraild` without startup live export routes, then loads a WASM observation plugin through:

```bash
actraild plugin load --manifest ... --plugin-config ... --instance wasm.observation-count
```

The fixture module exports the minimal AcTrail WASM observation ABI:

- `memory`
- `actrail_alloc(len) -> ptr`
- `actrail_plugin_init(ptr, len) -> status`
- `actrail_observation_consume(ptr, len) -> consumed_records`

Run:

```bash
python3 tests/plugins/wasm-observation/run_e2e.py
```
