# operation detail benchmark

对比六类操作在裸跑和 `actrailctl launch` 观测下的端到端耗时与内存：

- 固定编号文件的重复写入、读取；
- 轻量 Bash；
- 本地 HTTP/1.1 请求；
- 通过 Bash 重复调用 C 编译器；
- 每轮各执行一次上述操作的混合负载。

在仓库根目录运行：

```bash
sudo -E python3 scripts/bench/detail/__main__.py
```

默认先执行增量 `cargo build --release`，然后在 actraild 未运行时完成 bare
阶段，再用隔离目录生成刷新后的默认配置并启动真实 actraild，完成 observed
阶段。每个 workload 在目标进程内部用单调时钟包住 N 次操作；daemon 启动、
`actrailctl launch` 注册、目标启动和退出等待均不计入操作时间。

默认每种模式预热 1 次并丢弃，再测量 3 次。各类 N 均可独立覆盖：

| 操作 | 默认 N |
|---|---:|
| 文件写入 | 5000 |
| 文件读取 | 10000 |
| 轻量 Bash | 200 |
| 本地 HTTP | 1000 |
| Bash 编译 | 10 |
| 混合负载 | 12 |

```bash
sudo -E python3 scripts/bench/detail/__main__.py \
  --rounds 5 \
  --file-write-count 3000 \
  --file-read-count 15000 \
  --bash-light-count 300 \
  --network-count 300 \
  --bash-heavy-count 15 \
  --mixed-count 20
```

也可用 `--operations file-write network mixed` 只运行部分负载，或用 `--out`
保留逐轮 JSON。默认 N 以单项达到可采样的亚秒级时长为目标，完整默认测试通常
在几十秒内完成，不包含首次冷构建时间。

输出仅保留一张汇总表。时间列是目标进程内部正式轮次均值；`time Δ` 是 observed 相对
bare 的耗时变化。`bare MiB` 和 `observed MiB` 使用每轮独立 memory cgroup 的
峰值计数器（v2 `memory.peak` 或 v1 `memory.max_usage_in_bytes`），后者包含
workload 进程树，但不包含 `actrailctl` 和 actraild；
`daemon MiB` 单列显示同一 observed 窗口内采样到的 actraild 峰值 VmRSS。

为保证 bare 口径真实，启动时若发现任何 actraild 仍在运行会直接失败。脚本要求
root 和可写的 cgroup v1/v2 memory controller，并与 V2 regression 共用全局锁，避免
和其他真实 eBPF 测试并发。异常时保留工作目录；正常完成后自动停止 daemon 并清理。

仅校准各 workload 默认 N、不构建或操作 actraild 时可运行：

```bash
sudo -E python3 scripts/bench/detail/__main__.py --calibrate-only
```
