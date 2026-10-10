# WASM 发布后的网络访问控制性能

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

从仓库根目录执行，使用最新 release 二进制及已安装的官方 WASM 包：

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py run \
  --policy-source wasm --count 300 --rounds 3 --warmup 50
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py report
```

每个后端在同一工作进程内分别预热 allow/deny 各 50 次，随后各测量 3 轮，每轮
300 次。实验记录逐次延迟、吞吐、自身 getrusage CPU 和主动/被动上下文切换，
同时采集 daemon `/proc/PID/stat` CPU tick。真实 listener 计数和工作负载逐次 errno
验证策略实际执行；检查真实 Xiaoo 及后代线程 seccomp 状态，确保 LSM 路线无
seccomp/fanotify。

结果默认写入 `local/lsm_access_control/wasm-performance.json`、
`wasm-performance.md` 和 `wasm-benchmark-{seccomp,lsm}.log`。
`ACTRAIL_PLUGIN_DIR` 指定安装包目录，默认 `~/.actrail/plugins`；`--bin-dir` 指定
release 目录；`--backends lsm seccomp` 可交换测试顺序。实验持有公共 regression
锁，运行完成后停止自有 daemon、Web、provider 和 listener，并清理临时目录。
`--keep-work` 可保留现场。

静态规则对照：

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py run \
  --policy-source static --count 300 --rounds 3 --warmup 50
python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py report \
  --input local/lsm_access_control/performance.json \
  --output local/lsm_access_control/performance.md
```

此实验不加入默认回归，不设性能通过阈值。短测量窗口包含 Python、listener、观测和
调度成本；daemon CPU 受系统 tick 粒度限制，也没有等待全部异步观测尾部完成。
额外上下文切换可能由观测或其他调度竞争造成。结果仅代表当前网络策略条件，不能
推算文件/exec、逐请求 WASM 回调、规则发布并发或开启审计的性能。
