# 持续访问检查的性能测量

该测例使用同一组 release 二进制，对比关闭文件和网络控制与开启 LSM 控制时的持续访问成本。每组通过 `actrailctl init -f` 刷新默认配置，启动真实 Xiaoo，并让 Bash 工具执行工作负载。

文件描述符和 TCP 连接在计时前建立。计时操作是读取 1 字节、写入 1 字节和 `sendall` 发送 64 字节。每组保留相同的进程、网络和 socket bpf-copy 采集，关闭控制审计。测试检查文件写入结果与服务端收到的完整字节数。输出还记录 CPU 时间、上下文切换、真实 agent 祖先和每轮延迟。

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/retained/run.py run \
  --count 1000 \
  --rounds 5 \
  --warmup 100
```

入口使用现有 `target/release` 工件。`--bin-dir` 和 `--xiaoo` 可指定实际二进制。默认执行顺序为 `off lsm lsm off`，可通过 `--order` 调整。`--count` 默认 300，`--rounds` 默认 3，`--warmup` 默认 50。超时参数沿用上级性能测例的默认值，可通过 `--timeout`、`--command-timeout`、`--launch-timeout` 设置。

原始结果默认保存到 `local/lsm_access_control/retained.json`，可通过 `--output` 指定路径。临时配置、工作文件和测试服务在结束时清理。

## 本机测量

2026-11-05，aarch64，Linux `6.6.0-132.0.0.111.oe2403sp3.aarch64`。每种操作每组预热 100 次，测量 5 轮，每轮 1000 次。下表为各轮平均延迟的中位数，单位为微秒。

| 顺序 | 控制状态 | 读取 1 字节 | 写入 1 字节 | TCP 发送 64 字节 |
| --- | --- | ---: | ---: | ---: |
| 1 | off | 0.599 | 0.690 | 11.778 |
| 2 | LSM | 1.310 | 1.376 | 10.154 |
| 3 | LSM | 1.319 | 1.420 | 7.101 |
| 4 | off | 0.590 | 0.659 | 9.966 |

该负载中，持续文件检查使每次读写增加约 0.7 微秒。TCP 测量受发送缓冲、采集与调度影响，当前结果未分离出稳定的检查增量。

差值表示当前持续访问检查启用后的成本。结果包含 Python 调用成本和相同的采集开销，计时不含打开文件、建立连接、启动 agent 或发布规则。
