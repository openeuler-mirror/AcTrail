# Observation storage acceptance

```bash
python3 -m scripts.bench.payload.storage_independence.run \
  --bin-dir target/release \
  --output-dir local/bench/payload/noop-main
```

Refresh the default daemon configuration and select `storage.backend = "noop"`.
One real xiaoo session makes two requests to the repository local HTTPS MaaS
and executes one actual local MCP tool. A live OTLP/HTTP receiver must observe
both LLM request/response pairs and five successful MCP actions with one external
trace identity. Daemon launch/finalization logs establish completion independently
of historical storage. The main SQLite database and its WAL/SHM files must remain
absent before, during completion, and after daemon shutdown.

Select `--agent-kind opencode` for a real OpenCode session with two HTTPS requests
and an actual bash tool that copies the input into a checked result file. Its
online evidence requires both successful LLM exchanges and trace completion.
Use `--agent-bin` to select an exact agent executable.

`--config-patch scripts/bench/payload/configs/P.toml` applies the selected mode
file over refreshed defaults. Isolated runtime paths and the explicitly selected
`--storage-backend noop|sqlite` take precedence. The effective configuration and
selected patch path are saved with the result. SQLite acceptance checks the same
online evidence; it does not query the database to establish analysis completion.
Results label patched configuration as `fresh defaults + selected patch` and
record the explicitly selected storage backend separately.

Artifacts include effective configuration, agent and MaaS output, MCP execution
evidence, daemon logs, and online OTLP documents. The fixture owns isolated runtime
paths and stops only its own services. It performs no database queries, fault
injection, offline export, CPU matrix, or global installation. Dedicated small
quota coverage across TLS, socket, and stdio remains TODO.
