# cgroup trace resource metrics 与列存储回归

## 测试目标

验证 trace 专属 cgroup 的资源计量、生命周期收尾、重启恢复，以及资源样本的存储和查询一致性。

## 场景设计

用受控工作负载建立 trace 资源范围，分别执行正常退出、并行隔离、daemon 崩溃恢复、超时与范围丢失场景。以实际 cgroup 计数及持久化样本核对计量结果，再通过 viewer 和 HTTP 查询同一批数据。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储与查询 |
| 主机 | Linux、root、可管理的 cgroup 层级 | 隔离并计量 trace 资源 |
| cgroup v2 场景 | 统一 cgroup v2、systemd 254+ | 验证委托和 workload leaf |
| cgroup v1 场景 | cpu,cpuacct 与 memory 层级 | 验证 v1 trace 范围 |
| actrailweb | 可启动隔离 HTTP 实例 | 核对资源 API |

## 测试步骤摘要

1. 建立本轮专属 cgroup 根并启动 daemon。
2. 通过 controlled launch 启动工作负载，记录 trace 和资源范围。
3. 依次验证正常结束、隔离、崩溃恢复、收尾超时及范围丢失。
4. 按 trace 对照 SQLite、viewer 和 HTTP 资源样本。
5. 停止场景 daemon，在独立目录运行样本过期清理验收。
6. 清理本轮资源范围和工作目录。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 正常结束 | workload 范围清空 | 进程树资源得到计量，exact final 样本唯一 |
| 范围隔离 | 存在未追踪的满载进程 | 该进程资源不计入空闲 trace |
| 崩溃恢复 | daemon 被 SIGKILL 后重启 | 恢复范围并继续采样，旧 profile 和样本保持完整 |
| 超时与丢失 | 后代持续运行或范围被外部删除 | 超时产生 partial final 且后代继续运行，丢失范围有诊断并完成收尾 |
| 查询一致性 | 同一 trace 已持久化样本 | SQL、viewer、HTTP 的样本内容一致，时间桶和峰值正确 |
| 保留策略 | 独立清理场景完成 | 过期样本回收，新样本及仍被引用的元数据保留 |

## 快速运行

在仓库根目录执行。

```bash
python3.11 tests/e2e/regression/resource_metrics_cgroup/run_e2e.py
```

## 手动测试

此默认 V2 case 使用 release `actraild`、`actrailctl`、`actrailviewer` 和 `actrailweb` 验证：

- systemd 委托根与 `daemon/` leaf 满足 no-internal-process 规则；
- controlled launch 在 `exec` 前进入 trace `workload/` leaf，子进程继承同一边界；
- `memory.peak` 和 `pids.peak` 覆盖进程树，empty 后只持久化一个 exact final event；
- daemon 被 `SIGKILL` 后自动重启，恢复持久化 scope、继续采样并清理空 orphan；
- finalization timeout 生成 partial final event，且不终止仍在运行的后代。

在没有统一 cgroup v2 的宿主（如 openEuler 24.03 的纯 v1）上，同一 case 改跑自建 v1 trace 范围的场景
（`cgroup_v1.py`），使用开发或 release `actraild`、`actrailctl`、`actrailviewer`：

- exact：controlled launch 在 `exec` 前进入 `<挂载点>/<root>/traces/trace-<id>-<nonce>/workload`，
  满载 trace 的 CPU 秒数与内存峰值来自 trace 专属 cgroup；
- isolation：同一会话中未被追踪的满载进程不计入空闲 trace；
- restart：daemon 重启后接回持久化的范围并继续采样；
- timeout：进程迟迟不退出时，到 finalization 超时仍能收尾；
- lost：范围被外部删除时记录诊断，不写最终样本，也不阻塞。

v1 的 lost 场景在 daemon 暂停期间删除本轮测试自己的各层级范围。完成标记早于进程实际退出，
因此删除前等待 `cgroup.procs` 清空；控制器仍暂时返回 `EBUSY` 时，在同一超时预算内重试，
并保留已经删除的目录进度。超时会报告剩余目录、PID/TID 和最后的 `EBUSY`，其他删除错误直接失败。

v1 场景每次使用独立根目录 `actrail-e2e-<pid>`，不触碰已部署 daemon 的 `actrail/`，结束时删除；
工作负载通过 `actrailctl launch --host-ebpf disabled --seccomp-notify disabled` 启动，需要 root 以及 `cpu,cpuacct` 与 `memory` 层级。

结果按 `exact`、`isolation`（v1）、`restart`、`timeout`、`lost`、`column_storage`、`resource_api` 和 `retention`
分别显示；后续检查失败时保留此前通过的小项。

- `restart` 在崩溃前保存 profile 和完整 viewer 样本。重启后要求原 profile 内容不变，旧样本所有字段
  （包括 metadata）不变，样本数量继续增长，并沿用原有的唯一 final 样本检查。
- `column_storage` 检查新 schema、profile/boot 引用、extra BLOB 类型；按指定 trace 对照 SQL 和真实
  viewer 的完整行数、顺序、时间戳、process identity 和 18 个指标列。
- `resource_api` 在测试自己的配置上启动隔离 Web 实例，使用动态监听端口。两条真实 trace 的
  `/resources` 必须与完整 viewer 样本逐字段一致，并验证最新一条的 limit；`/resource-timeline`
  验证完整窗口、子窗口、空窗口、每桶样本数及内存/PID 峰值，并拒绝非法参数。
- `retention` 在上述检查通过、cgroup 场景的 daemon 停止后，使用独立的 `retention/` 工作目录、配置和
  SQLite 启动自己的真实 daemon，复用 [`resource_storage_retention`](../resource_storage_retention/README.zh.md)
  的清理验收。该小项继续展开 `expired_samples`、`fresh_samples`、`profiles_and_boots`、`metadata_keys`
  和 `legacy_table` 五项结果，任一失败都会使整个 case 失败。cgroup 前置条件不满足时随整个 case 跳过。

这些检查走真实 daemon、SQLite、viewer 和 HTTP。编解码 round-trip 单测仍只在仓库外本地验证，不提交。

v2 场景需要 root、统一 cgroup v2、systemd 254+（`DelegateSubgroup=`）以及 release binaries。
不满足外部宿主条件时返回明确 `SKIPPED`；必需的 privileged CI lane 应把 skip 当成失败。
