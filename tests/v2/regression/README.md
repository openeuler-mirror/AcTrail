# V2 regression tests

运行全部测例：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py
```

列出或选择测例：

```bash
python3.11 tests/v2/regression/test_all.py --list
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_codex_llm
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_pi_llm
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_qodercli_llm
sudo -E python3.11 tests/v2/regression/test_all.py --case semantic_action_boundaries
sudo -E python3.11 tests/v2/regression/test_all.py --case otel_jsonl_action_filter
sudo -E python3.11 tests/v2/regression/test_all.py --case plugin_activity_anomaly
sudo -E python3.11 tests/v2/regression/test_all.py --fail-fast --no-cleanup
```

`--case` 可重复指定。公共参数包括：

- `--bin-dir`：AcTrail release 二进制目录，也可通过 `ACTRAIL_BIN_DIR` 设置。
- `--color {auto,always,never}`：控制彩色结果符号。
- `--log-dir`：每个测例独立日志的目录，默认为
  `/tmp/actrail-regression/logs`，也可通过 `ACTRAIL_TEST_LOG_DIR` 设置。
- `--work-root`：runner 为每个 `TestDefinition` 注入独立 `work_dir` 的根目录，
  默认为 `/tmp/actrail-regression`，也可通过 `ACTRAIL_TEST_WORK_ROOT` 设置。
- `--cleanup/--no-cleanup`：是否在测例结束后调用 case cleanup hook，并删除该
  case 的 workspace 和 runner log；默认启用清理。
- `--lock-path`：保护全局 daemon、socket、数据库和 Web 端口的跨进程锁文件，
  默认为 `/run/lock/actrail-v2-regression.lock`，也可通过
  `ACTRAIL_TEST_LOCK_PATH` 设置。
- `--lock-timeout-seconds`：等待同一 Python 进程内其他 regression 线程以及
  其他 regression 进程完成的总时限，默认 900 秒，也可通过
  `ACTRAIL_TEST_LOCK_TIMEOUT_SECONDS` 设置。
- `--lock-poll-seconds`：等待锁时的轮询间隔，默认 1 秒，也可通过
  `ACTRAIL_TEST_LOCK_POLL_SECONDS` 设置。
- `--fail-fast`：首个测例失败后立即停止，不再运行后续测例。与
  `--no-cleanup` 组合可保留失败测例的 workspace、runner log 和最后一次
  runtime 数据库现场。

runner 在创建 suite context 时获取一次全局文件锁，整套 selected cases 完成后才释放。
持有进程退出或崩溃时锁由内核自动释放；锁文件本身会保留，但不表示仍被占用。后启动的
runner 会显示进程或线程持有者并等待；进程内 lease 与文件锁共用上述总时限，超时后会
明确报告测试未启动，并在执行任何 daemon stop/clean 之前失败。

`test_all.py` 中每个测例的名称和最终结果显示在同一行，stdout、stderr、命令输出
及详细检查结果统一写入 `<log-dir>/<case>.log`。

单个测例通过其目录中的 `run_e2e.py` 独立运行时，会在终端实时显示 stdout、
stderr 和完整检查明细，同时仍由公共框架保存对应日志。

各测例的 Quick Run、步骤摘要和可复制执行的手动测试流程：

| 测例 | 中文操作文档 |
| --- | --- |
| `probe_claude_llm` | [`probe_claude_llm/README.zh.md`](probe_claude_llm/README.zh.md) |
| `probe_codex_llm` | [`probe_codex_llm/README.zh.md`](probe_codex_llm/README.zh.md) |
| `probe_pi_llm` | [`probe_pi_llm/README.zh.md`](probe_pi_llm/README.zh.md) |
| `probe_qodercli_llm` | [`probe_qodercli_llm/README.zh.md`](probe_qodercli_llm/README.zh.md) |
| `probe_xiaoo_llm` | [`probe_xiaoo_llm/README.zh.md`](probe_xiaoo_llm/README.zh.md) |
| `semantic_action_boundaries` | [`semantic_action_boundaries/README.zh.md`](semantic_action_boundaries/README.zh.md) |
| `otel_jsonl_action_filter` | [`otel_jsonl_action_filter/README.zh.md`](otel_jsonl_action_filter/README.zh.md) |
| `plugin_activity_anomaly` | [`activity_anomaly/README.zh.md`](activity_anomaly/README.zh.md) |
