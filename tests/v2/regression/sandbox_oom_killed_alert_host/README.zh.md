# Host `sandbox.resource.oom_killed` focused 回归

该可选 case 只验证一次受控 memory-cgroup OOM 与对应的
`sandbox.resource.oom_killed`（`critical`）公开告警投递。它不属于默认回归集合，
不读取 Kata 机器本地 profile，也不依赖 VMM、xiaoO、LLM provider 或 actrailweb。

case 会在内部创建 `comm=actrail-root` 的临时命名根并记录稳定谱系 marker，
不需要仓库外的 helper binary 或 shell 函数。

## 快速运行

从仓库根目录显式运行：

```bash
PYTHONDONTWRITEBYTECODE=1 \
deploy/virtual-container/host/run-v2-tests.sh \
  --no-profile \
  --case sandbox_oom_killed_alert_host \
  --color never \
  --fail-fast
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 `CARGO_HOME`、
`RUSTUP_HOME`、`~/.cargo/bin`、`~/.local/bin` 和现有 `PATH`。显式
`--no-profile` 表示这个宿主 case 不读取 Kata 本机 profile。

如果绕过该入口直接调用 `tests/v2/regression/test_all.py`，调用方仍须以 root
运行，并自行确保 root 环境能够找到 Python、Cargo 和 Rustup。

## 前置条件

- Linux root、`/sys/kernel/btf/vmlinux`、`/dev/vsock` 和 `vsock_loopback`；
- 可在 memory cgroup v1 或 v2 下创建并限制一个测试子 cgroup；宿主存在
  active swap 时还必须提供对应的 per-cgroup swap limit；
- 当前 checkout 的 release 二进制。公共 runner 启动时仍会执行
  `scripts/install-release.sh`，因此 root 环境必须能找到 Cargo；
- `python3`（3.11+）、`awk` 和标准 `/bin/sh`。

case 会先做 cgroup delegation 探测。外部内核能力不足时返回 `SKIPPED`；release 或
仓库资产缺失时返回 `FAILED`。

## 安全边界和验收

注入器只把一个 Python allocator 放入测试专属的 32 MiB memory cgroup。宿主存在
active swap 时必须成功把该 cgroup 的 swap 限制为 0（v2）或把 memory+swap 总量限制
为 32 MiB（v1），否则 case 在注入前 `SKIPPED`；不会向宿主根 cgroup 施加内存压力。
它要求 allocator 被 SIGKILL（退出码 137），同时要求该 cgroup 与 `/proc/vmstat` 的
`oom_kill` 计数增长。超时或异常路径会终止整个注入进程组、reap allocator，并验证
测试 cgroup 已删除。

主断言来自 alert-proxy 的公开 subscriber：同一 victim PID 必须恰好收到一条
`sandbox.resource.oom_killed`，其 `victim_pid`、`victim_comm=python3`、
`attribution=monitored` 和稳定 `actrail-root` marker 必须与注入证据一致。独立 Alert
SQLite 仅用于验证已提交记录与公开 delivery 完全一致。

入口配置使用 `SANDBOX_OOM_KILLED_ALERT_HOST_E2E_` 前缀，可覆盖
`VSOCK_PORT`、`READY_TIMEOUT_SECONDS`、`RUNTIME_TIMEOUT_SECONDS` 和
`ROOT_DISCOVERY_SETTLE_SECONDS`。OOM workload 的 cgroup 限额与证据条件由场景固定，
不作为环境调参暴露。root settle 配置是下限；case 始终至少等待当前 release 配置的
两个 `root_refresh_interval`，再创建 OOM victim。
