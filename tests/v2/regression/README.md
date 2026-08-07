# V2 regression tests

运行全部测例：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py
```

运行全部虚拟容器测例：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case virtual_container \
  --case virtual_container_xiaoo_concurrency
```

`test_all.py` 会自动加载存在的
`local/kata/v2-test-profile.json`。该机器本地文件保存 runtime、VMM 配置、镜像、
workload bundle 和 xiaoO 路径，因此正常验收不需要再粘贴环境变量。显式 shell
环境变量优先于 profile；可用 `--profile <path>` 选择其他 profile，或用
`--no-profile` 禁用自动加载。profile 不得保存密码、API key 或其他凭据。

虚拟容器首次部署应优先使用
`deploy/virtual-container/host/run-v2-tests.sh`。`local/kata/` 是 checkout-local 的
Git 忽略资产：必须先在最终 checkout 拉取目标分支，再从同一目录运行 artifact
preparer。新 worktree 不会继承旧 profile；默认 profile 缺失时该包装脚本会在 sudo
和 runner 启动前给出明确错误。完整步骤见
[`virtual_container/README.zh.md`](virtual_container/README.zh.md)。

列出或选择测例：

```bash
python3.11 tests/v2/regression/test_all.py --list
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_claude_mcp
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_codex_mcp
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_codex_llm
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_pi_llm
sudo -E python3.11 tests/v2/regression/test_all.py --case probe_qodercli_llm
sudo -E VIRTUAL_CONTAINER_E2E_SCOPE=contracts \
  python3.11 tests/v2/regression/test_all.py --case virtual_container
sudo -E python3.11 tests/v2/regression/test_all.py --case container_auto
sudo -E python3.11 tests/v2/regression/test_all.py --case container_agent_xiaoo
sudo -E python3.11 tests/v2/regression/test_all.py --case semantic_action_boundaries
sudo -E python3.11 tests/v2/regression/test_all.py --case otel_jsonl_action_filter
sudo -E python3.11 tests/v2/regression/test_all.py --case plugin_activity_anomaly
sudo -E python3.11 tests/v2/regression/test_all.py --case tool_consecutive_failure_alert
sudo -E python3.11 tests/v2/regression/test_all.py --fail-fast --no-cleanup
```

`virtual_container` 默认使用 `VIRTUAL_CONTAINER_E2E_SCOPE=auto`：contracts 通过后，
无可读写 `/dev/kvm` 的主机会以明确的 `SKIPPED` 结果停止；具备 KVM 的主机会继续完整
runtime 矩阵。仅在需要强制行为时显式设置 `contracts` 或 `all`。

`--case` 可重复指定。公共参数包括：

- `--case`：选择一个测例，可重复指定以运行多个测例。
- `--profile/--no-profile`：选择或禁用机器本地测试配置。
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

runner 在创建 regression context 时获取一次全局文件锁，全部 selected cases 完成后才释放。
持有进程退出或崩溃时锁由内核自动释放；锁文件本身会保留，但不表示仍被占用。后启动的
runner 会显示进程或线程持有者并等待；进程内 lease 与文件锁共用上述总时限，超时后会
明确报告测试未启动，并在执行任何 daemon stop/clean 之前失败。

取得全局锁后，runner 会在 singleton 生命周期内执行一次
`bash scripts/install-release.sh`，成功后才进入任何测例。安装脚本从仓库根目录运行，
会让 Cargo 检查并刷新 release 和官方 WASM 插件、安装或检查构建依赖，并覆盖
`/usr/local/bin` 与 `$ACTRAIL_PLUGIN_DIR`（默认为 `$HOME/.actrail/plugins`）中的
AcTrail 产物。安装失败属于启动失败，runner 不会进入测例。`--bin-dir` 仍只控制测例
使用的二进制目录，不改变安装脚本的目标目录。

`test_all.py` 在 TTY 中只用短 step 名原地刷新每个测例的当前步骤和最终结果；step
说明、stdout、stderr、命令输出及详细检查结果统一写入
`<log-dir>/<case>.log`。

单个测例通过其目录中的 `run_e2e.py` 独立运行时，会在终端实时显示 stdout、
stderr 和完整检查明细，同时仍由公共框架保存对应日志。

各测例的 Quick Run、步骤摘要和可复制执行的手动测试流程：

| 测例 | 中文操作文档 |
| --- | --- |
| `probe_claude_llm` | [`probe_claude_llm/README.zh.md`](probe_claude_llm/README.zh.md) |
| `probe_claude_mcp` | [`probe_claude_mcp/README.zh.md`](probe_claude_mcp/README.zh.md) |
| `probe_codex_llm` | [`probe_codex_llm/README.zh.md`](probe_codex_llm/README.zh.md) |
| `probe_codex_mcp` | [`probe_codex_mcp/README.zh.md`](probe_codex_mcp/README.zh.md) |
| `probe_pi_llm` | [`probe_pi_llm/README.zh.md`](probe_pi_llm/README.zh.md) |
| `probe_qodercli_llm` | [`probe_qodercli_llm/README.zh.md`](probe_qodercli_llm/README.zh.md) |
| `probe_xiaoo_llm` | [`probe_xiaoo_llm/README.zh.md`](probe_xiaoo_llm/README.zh.md) |
| `virtual_container` | [`virtual_container/README.zh.md`](virtual_container/README.zh.md) |
| `virtual_container_xiaoo_concurrency` | [`virtual_container_xiaoo_concurrency/README.zh.md`](virtual_container_xiaoo_concurrency/README.zh.md) |
| `container_auto` | [`container_auto/README.zh.md`](container_auto/README.zh.md) |
| `container_agent_xiaoo` | [`container_agent_xiaoo/README.zh.md`](container_agent_xiaoo/README.zh.md) |
| `semantic_action_boundaries` | [`semantic_action_boundaries/README.zh.md`](semantic_action_boundaries/README.zh.md) |
| `otel_jsonl_action_filter` | [`otel_jsonl_action_filter/README.zh.md`](otel_jsonl_action_filter/README.zh.md) |
| `plugin_activity_anomaly` | [`activity_anomaly/README.zh.md`](activity_anomaly/README.zh.md) |
| `tool_consecutive_failure_alert` | [`tool_consecutive_failure_alert/README.zh.md`](tool_consecutive_failure_alert/README.zh.md) |
