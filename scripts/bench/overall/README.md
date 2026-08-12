# overall replay benchmark

对比两种方式回放录制剧本的开销（wall time / CPU / 峰值 RSS）：

1. 裸跑：replay server + xiaoo 直接运行；
2. actrail 托管：replay server + actraild，xiaoo 通过 `actrailctl launch` 运行。

```bash
python3 scripts/bench/overall/__main__.py \
  --scenario recorded/recorded-xiaoo-common-20260809032236-3e287 \
  --rounds 11
```

`--scenario` 必填，不记得有哪些剧本时先查询（只查询不运行，直接调用
MaaS 服务端自己的 `ScenarioRegistry`，保证列表与加载口径一致）：

```bash
python3 scripts/bench/overall/__main__.py --list-scenarios
```

未指定 `--scenario` 直接运行时会报错并附带可选剧本列表。agent 二进制默认
从 `PATH` 解析（xiaoo / opencode），特殊路径用 `--agent-binary` 覆盖。

一轮 A/B 测试 = xiaoo 跑完整份录制剧本（`max_turns` 默认等于剧本全部轮次，
tool 队列 + message 队列的总和），不截断任务。

启动时（准备 actraild 前）会先执行 `cargo build --release`（增量构建，
复用已有产物，不做 clean 重编译），并在构建前后各读取一次 git commit id：
若构建前后 id 不一致则启动失败。随后打印当前 commit id 与 title。
构建超时可用 `--build-timeout-seconds` 调整（默认 3600 秒）。

`--rounds` 表示每个 case 计入汇总的正式轮数；脚本会在此之外为 bare 和
actrail 各运行一次独立 prewarm 并丢弃。因此 `--rounds 5` 时每个 case
实际运行 6 次，汇总统计后 5 次正式运行。

指标基于 `/proc` 对进程树采样：wall 时间、进程树累计 CPU（user+sys）、
峰值 RSS；case 2 额外统计 actraild 自身的 CPU/RSS。结果打印并写入
`out/bench-overall-<时间戳>.json`。

报告 JSON 顶部记录元信息：`commit.id` / `commit.title`（本次构建校验后的
commit）、`scenario`（所用剧本）、`agent`（所用 agent）。

actraild 的 CPU/RSS 按"本次运行增量"统计：基线在 daemon 完成
`host_ebpf_preflight` 之后、launch 之前采集，因此启动/preflight 的一次性
开销不计入单次运行对比。
