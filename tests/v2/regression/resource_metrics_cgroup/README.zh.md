# cgroup v2 trace resource metrics 回归

此可选 V2 case 使用 release `actraild`、`actrailctl` 和 `actrailviewer` 验证：

- systemd 委托根与 `daemon/` leaf 满足 no-internal-process 规则；
- controlled launch 在 `exec` 前进入 trace `workload/` leaf，子进程继承同一边界；
- `memory.peak` 和 `pids.peak` 覆盖进程树，empty 后只持久化一个 exact final event；
- daemon 被 `SIGKILL` 后自动重启，恢复持久化 scope、继续采样并清理空 orphan；
- finalization timeout 生成 partial final event，且不终止仍在运行的后代。

测试需要 root、统一 cgroup v2、systemd 254+（`DelegateSubgroup=`）以及 release binaries。
不满足外部宿主条件时返回明确 `SKIPPED`；必需的 privileged CI lane 应把 skip 当成失败。

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --no-profile --case resource_metrics_cgroup
```
