# V2 regression tests

运行全部测例：

```bash
sudo -E python3 tests/v2/regression/test_all.py
```

列出或选择测例：

```bash
python3 tests/v2/regression/test_all.py --list
sudo -E python3 tests/v2/regression/test_all.py --case probe_codex_llm
sudo -E python3 tests/v2/regression/test_all.py --case probe_pi_llm
sudo -E python3 tests/v2/regression/test_all.py --case probe_qodercli_llm
```

`--case` 可重复指定。公共参数包括：

- `--bin-dir`：AcTrail release 二进制目录，也可通过 `ACTRAIL_BIN_DIR` 设置。
- `--color {auto,always,never}`：控制彩色结果符号。
- `--log-dir`：每个测例独立日志的目录，默认为
  `/tmp/actrail-v2-regression`，也可通过 `ACTRAIL_TEST_LOG_DIR` 设置。

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
