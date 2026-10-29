# cgroup trace resource metrics 与列存储回归

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

```bash
python3.11 tests/v2/regression/resource_metrics_cgroup/run_e2e.py
```
