# Host `sandbox.resource.oom_killed` focused 回归

## 测试目标

验证一次受控 memory-cgroup OOM 能产生且投递一条正确归属的 critical 告警。

## 场景设计

创建命名根进程和测试专属 32 MiB memory cgroup，把单个 Python allocator 放入其中触发内核 OOM。以进程退出和内核计数确认 OOM，再核对公开 subscriber 与独立 Alert SQLite。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储与查询 |
| 主机 | Linux、root、BTF、/dev/vsock、vsock_loopback | 采集和传输 OOM 事件 |
| memory cgroup | 可创建并限制 v1 或 v2 子 cgroup，存在 active swap 时支持独立 swap 限制 | 约束测试 allocator |
| 工具 | awk 和 /bin/sh | 运行受控负载 |

## 测试步骤摘要

1. 探测 cgroup delegation 与 swap 限制能力，条件不足时跳过。
2. 启动本轮命名根和观测链路，等待根发现稳定。
3. 设置 32 MiB 测试 cgroup，启动单个 Python allocator。
4. 确认 SIGKILL 和 OOM 计数增长。
5. 按 victim PID 核对 subscriber 告警与数据库。
6. 终止并回收负载进程组，验证测试 cgroup 删除。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 真实 OOM | allocator 位于受限 cgroup | 退出码 137，cgroup 和 /proc/vmstat 的 oom_kill 计数增长 |
| 唯一投递 | subscriber 收到该 victim 的告警 | 恰好一条 sandbox.resource.oom_killed，等级 critical |
| 身份归属 | 对照注入证据 | victim PID、python3、monitored 和 actrail-root 稳定 marker 匹配 |
| 持久化一致性 | 告警已提交 | 独立 Alert SQLite 与公开投递内容一致 |
| 清理 | 场景成功、超时或异常结束 | allocator 被回收，测试 cgroup 删除 |

## 快速运行

从仓库根目录显式运行：

```bash
PYTHONDONTWRITEBYTECODE=1 \
deploy/virtual-container/host/run-v2-tests.sh \
  --case sandbox_oom_killed_alert_host \
  --color never \
  --fail-fast
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 `CARGO_HOME`、
`RUSTUP_HOME`、`~/.cargo/bin`、`~/.local/bin` 和现有 `PATH`。该宿主 case 不需要本机 profile。

如果绕过该入口直接调用 `tests/e2e/regression/test_all.py`，调用方仍须以 root
运行，并自行确保 root 环境能够找到 Python、Cargo 和 Rustup。

## 手动测试

### 前置条件

- Linux root、`/sys/kernel/btf/vmlinux`、`/dev/vsock` 和 `vsock_loopback`；
- 可在 memory cgroup v1 或 v2 下创建并限制一个测试子 cgroup；宿主存在
  active swap 时还必须提供对应的 per-cgroup swap limit；
- 当前 checkout 的 release 二进制。公共 runner 启动时仍会执行
  `scripts/install-release.sh`，因此 root 环境必须能找到 Cargo；
- `python3`（3.11+）、`awk` 和标准 `/bin/sh`。

case 会先做 cgroup delegation 探测。外部内核能力不足时返回 `SKIPPED`；release 或
仓库资产缺失时返回 `FAILED`。

### 注入范围与详细验收

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
