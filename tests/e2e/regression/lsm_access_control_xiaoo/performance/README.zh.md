# WASM 发布后的网络访问控制性能

## 测试目标

测量真实 Agent 网络访问在 seccomp 与 BPF LSM 后端下的成本，并核验被测操作实际执行对应策略。

## 场景设计

通过官方 WASM 发布精确 endpoint allow/deny 规则，再让真实 XiaoO 的 Bash 工作负载执行 TCP connect 与 close。两种后端使用相同观测配置，分别记录发布耗时和预热后的访问耗时。

使用真实 Xiaoo 的 Bash 工具执行 IPv4 TCP connect+close 工作负载，对比 seccomp
用户态裁决与 BPF LSM 内核裁决。两种配置均刷新默认配置，采用相同 bpf-copy
观测并关闭网络审计、文件治理和命令治理。

默认通过已安装的官方 `network-policy-dynamic` WASM 包发布两条精确 endpoint
allow/deny 规则。静态规则文件为空，插件只获得这两条规则所需的 grant。
实验先校验空规则默认 allow，经真实 Web Configuration Validate/Update 后核验
规则、owner 和 source_revision，再由真实 Xiaoo 触发网络访问。

WASM 负责规则发布；发布后的每次 connect 由对应后端执行规则。计时窗口不含
发布、agent 启动和预热；另记录一次 Update 墙钟时间，其中包括 curl、HTTP、Web、
WASM 和宿主规则发布开销。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | release 产物和场景所需的官方插件 | 运行真实治理和观测 |
| 主机 | Linux、root、eBPF 与 BPF LSM | 执行内核访问控制 |
| XiaoO | 可用的真实 CLI | 驱动实际工作负载 |
| 工作负载服务 | 可绑定的本地 listener | 核对真实连接或接收字节 |

## 测试步骤摘要

1. 启动真实 TCP listener，为 seccomp 与 LSM 两组刷新默认配置，保持相同 bpf-copy 观测设置。
2. 检查空规则默认 allow，通过 Web Validate/Update 发布精确 endpoint allow/deny 规则，记录 owner、source revision 和发布耗时。
3. 由真实 XiaoO 启动 TCP connect+close 工作负载，先分别预热 allow 与 deny，再执行多轮测量。
4. 用逐次 errno 与 listener 计数核对规则效果，检查 Agent 祖先、线程 Seccomp 状态及所选后端。
5. 保存逐次延迟、吞吐、工作负载 CPU、上下文切换及 daemon CPU tick，停止服务并保留测量结果。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 规则效果 | 发布 allow 与 deny | 实际 errno 与 listener 收到的连接相符 |
| 后端身份 | 分别运行 seccomp 和 LSM | Agent 祖先、线程和后端状态符合选择 |
| 测量结果 | 完成预热与多轮测量 | 记录逐次延迟、吞吐、CPU 和上下文切换，结果保留原始样本 |

## 快速运行

从仓库根目录执行。

```bash
python3 tests/e2e/regression/test_all.py --case lsm_access_control_xiaoo_performance --no-cleanup
```

每个后端在同一工作进程内分别预热 allow/deny 各 50 次，随后各测量 3 轮，每轮
300 次。实验记录逐次延迟、吞吐、自身 getrusage CPU 和主动/被动上下文切换，
同时采集 daemon `/proc/PID/stat` CPU tick。真实 listener 计数和工作负载逐次 errno
验证策略实际执行；检查真实 Xiaoo 及后代线程 seccomp 状态，确保 LSM 路线无
seccomp/fanotify。

结果写入测例工作目录的 `measurements.json` 和 `measurements.md`，使用 `--no-cleanup` 保留。
`--bin-dir` 指定 release 目录。参数使用 `LSM_PERFORMANCE_E2E_` 前缀的环境变量。

| 后缀 | 默认值 | 用途 |
| --- | --- | --- |
| `XIAOO` | PATH 查找 | Agent 路径 |
| `POLICY_SOURCE` | wasm | wasm 或 static |
| `BACKENDS` | seccomp lsm | 后端执行顺序 |
| `COUNT` | 300 | 每轮操作次数 |
| `ROUNDS` | 3 | 轮数 |
| `WARMUP` | 50 | 预热次数 |
| `TIMEOUT` | 15 | 工作负载超时秒数 |
| `COMMAND_TIMEOUT` | 60 | 命令超时秒数 |
| `LAUNCH_TIMEOUT` | 240 | Agent 超时秒数 |
| `PEER_POLL_SECONDS` | 0.05 | listener 轮询间隔 |
| `BACKLOG` | 4096 | listener backlog |

`ACTRAIL_PLUGIN_DIR` 指定安装包目录，默认 `~/.actrail/plugins`。
设置 `LSM_PERFORMANCE_E2E_POLICY_SOURCE=static` 执行静态规则对照。

此实验不加入默认回归，不设性能通过阈值。短测量窗口包含 Python、listener、观测和
调度成本；daemon CPU 受系统 tick 粒度限制，也没有等待全部异步观测尾部完成。
额外上下文切换可能由观测或其他调度竞争造成。结果仅代表当前网络策略条件，不能
推算文件/exec、逐请求 WASM 回调、规则发布并发或开启审计的性能。
