# Storage CPU comparison

Ordinary pipe/FIFO and Unix socket observation is disabled in the mode configurations and ablation presets.

Run a real agent against local HTTPS MaaS. Defaults use xiaoo with 120 turns, 128 input bytes and TPOT 3 ms. Each mode warms up once and measures three runs. `configs/P.toml` and `configs/C.toml` define the collection modes; the selected storage backend is appended to the isolated configuration. Both backends use the same binary directory and mode files.

The binary directory must contain `commit.txt` with the full commit passed through `--expected-commit`. Results record that commit, the fixed artifact paths and the runner commit. Run each command serially, using a fresh output directory under `local/`:

```sh
sudo python scripts/bench/payload/storage_cpu/run.py --bin-dir <BINARY_DIR> --expected-commit <COMMIT_ID> --backend sqlite --out <SQLITE_OUTPUT_DIR>
sudo python scripts/bench/payload/storage_cpu/run.py --bin-dir <BINARY_DIR> --expected-commit <COMMIT_ID> --backend noop --out <NOOP_OUTPUT_DIR>
```

Use `--agent-kind opencode --agent-turns 4` for the measured OpenCode short workload (four requests and three tools). Use `--agent-bin` to select an executable explicitly. Each command measures bare, P and C. Keep workload parameters identical for both backends. Configuration and workload overrides are recorded in the output. The OpenCode short workload takes about 3.6–5.1 seconds per sample; it is shorter than the general 5–10 second benchmark target.

Report extra CPU as `(task CPU + daemon CPU - bare CPU) / bare CPU * 100`, using each run's measured bare CPU. Summarize existing results without rerunning agents:

```sh
python -m scripts.bench.payload.storage_cpu.compare --sqlite <SQLITE_OUTPUT_DIR> --noop <NOOP_OUTPUT_DIR> --out <REPORT_OUTPUT_DIR>
```

Task CPU uses `wait4` user and system time, including reaped descendants and the observed launch wrapper. Daemon CPU uses `/proc` counters from command launch through the matching trace finalization log entry. Startup and shutdown are outside that window. Both storage backends use the same log-based completion condition. SQLite coverage queries run after the final CPU sample.

The fixture validates actual MaaS requests and tool outputs for every run. SQLite additionally verifies stored LLM request/response counts and trace health. NoOp verifies launch/finalization and absence of a SQLite database; online capture coverage requires the independent acceptance evidence and cannot be inferred from lifecycle logs. The benchmark saves `results.json`, effective configurations and owned runtime artifacts, and stops only its own daemon.
