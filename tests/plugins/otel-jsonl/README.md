# OTEL JSONL Plugin E2E

Validates that live OTEL JSONL output works through the observation-consumer plugin path and that storage-backed `actrailviewer export-otel` remains compatible.

Run:

```bash
cargo build --release --bin actraild --bin actrailctl --bin actrailviewer
python3 tests/plugins/otel-jsonl/run_e2e.py
```

Expected evidence:

- `plugin_otel_jsonl_spans=<N>` is printed with `N > 0`.
- `plugin_otel_jsonl_semantic_actions=<N>` is printed with `N > 0`.
- `/tmp/actrail-plugin-otel-jsonl/live-spans.otlp.jsonl` exists and contains the marker `ACTRAIL_PLUGIN_OTEL_JSONL_E2E`.
- `/tmp/actrail-plugin-otel-jsonl/exported.otlp.json` exists and contains the same marker.
