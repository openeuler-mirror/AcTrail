# 持续访问检查的性能测量

## 测试目标

测量已打开文件和已建立连接上的持续访问检查成本，并验证读取、写入和发送实际完成。

## 场景设计

在同一组 release 产物下，对照关闭控制与开启 LSM。文件描述符和 TCP 连接在计时前建立，工作负载由真实 XiaoO 启动，各组保留相同观测配置。

该测例使用同一组 release 二进制，对比关闭文件和网络控制与开启 LSM 控制时的持续访问成本。每组通过 `actrailctl init -f` 刷新默认配置，启动真实 Xiaoo，并让 Bash 工具执行工作负载。

文件描述符和 TCP 连接在计时前建立。计时操作是读取 1 字节、写入 1 字节和 `sendall` 发送 64 字节。每组保留相同的进程、网络和 socket bpf-copy 采集，关闭控制审计。测试检查文件写入结果与服务端收到的完整字节数。输出还记录 CPU 时间、上下文切换、真实 agent 祖先和每轮延迟。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | release 产物和场景所需的官方插件 | 运行真实治理和观测 |
| 主机 | Linux、root、eBPF 与 BPF LSM | 执行内核访问控制 |
| XiaoO | 可用的真实 CLI | 驱动实际工作负载 |
| 工作负载服务 | 可绑定的本地 listener | 核对真实连接或接收字节 |

## 测试步骤摘要

1. 按配置顺序运行关闭控制与开启 LSM 的对照组，每组刷新默认配置并启动真实 XiaoO。
2. 由 Bash 工作负载打开文件并建立 TCP 连接，在计时前完成描述符与连接准备。
3. 完成预热后，分轮测量读取 1 字节、写入 1 字节和通过已有连接发送 64 字节。
4. 核对实际文件写入与服务端接收字节数，记录真实 Agent 祖先、CPU 时间、上下文切换和每轮延迟。
5. 保存 measurements.json，停止服务并按运行参数保留结果。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 工作负载 | 重复读取、写入和发送 | 文件写入正确，服务端收到完整字节数 |
| 测量范围 | 描述符与连接已建立 | 计时覆盖持续访问操作，记录每轮延迟 |
| 可核对性 | 全部测量完成 | 保存 CPU、上下文切换和真实 Agent 祖先信息 |

## 快速运行

从仓库根目录执行。

```bash
LSM_RETAINED_PERFORMANCE_E2E_COUNT=1000 \
LSM_RETAINED_PERFORMANCE_E2E_ROUNDS=5 \
LSM_RETAINED_PERFORMANCE_E2E_WARMUP=100 \
python3 tests/e2e/regression/test_all.py --case lsm_access_control_xiaoo_retained_performance --no-cleanup
```

参数使用 `LSM_RETAINED_PERFORMANCE_E2E_` 前缀。`ORDER` 默认 `off lsm lsm off`，
可用空格分隔的 `off`、`empty`、`lsm` 指定执行顺序。
`XIAOO`、`COUNT`、`ROUNDS`、`WARMUP`、`TIMEOUT`、`COMMAND_TIMEOUT`、`LAUNCH_TIMEOUT`
的默认值与上级性能文档一致。`--bin-dir` 指定 release 工件。
结果保存在测例工作目录的 `measurements.json`。

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
