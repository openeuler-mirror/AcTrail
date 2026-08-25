# Sandbox Resource Alert Host 回归

该场景在无需 VMM 的主机环境验证采集、传输、落库和告警转发组件通路。

实际运行路径如下：

```text
真实 xiaoO 与命名根进程
  → actrail-sb
  → Host native AF_VSOCK loopback
  → actrail-vsock-gateway
  → actraild
  → sandbox-resource-alert
  → 独立 Sandbox Alert SQLite
  → actraild-alert-proxy
  → 外部 subscriber
```

场景使用真实当前 release 二进制和真实 xiaoO。

`actrail-sb init` 从当前 release 刷新默认配置。

该配置只包含测试目录内的 control socket、实例锁和采集参数，
不保存 VSOCK CID 与端口。

场景先以 `actrail-sb daemon --config` 启动采集 daemon，
等待 control UDS 就绪，并跨越多个资源采样周期确认 daemon 存活。

未建立 VSOCK 连接时，采集结果不会进入独立 Evidence SQLite，
也不会产生 Sandbox Alert SQLite 记录。

Host native VSOCK gateway 就绪后，场景执行真实
`actrail-sb connect --control-socket ... --host-cid ... --port ...`，
成功后才启动工作负载并验收完整告警通路。

命名根进程的子进程执行文件读取和写入。

场景启动真实 `actrailweb`，通过 plugin config HTTP API 把可用内存阈值降到非风险范围，等待活动插件消费新资源快照，再把阈值提高到风险范围。
同一个 plugin instance 必须产生新的可用内存风险告警，更新后的 JSON 必须写回原配置文件。

真实 xiaoO 的一个 Bash tool call 创建测试专属 memory cgroup，只把一个 Python 分配进程放入 32 MiB 限额并触发真实内核 OOM。
测试根进程和宿主其他进程不进入该 cgroup。

最终要求独立告警数据库与 subscriber 同时出现以下告警：

- 可用内存风险
- OOM killed
- 进程高读取量
- 进程高写入量

读写告警必须匹配命名根进程的 PID、启动时钟和进程名，
并且观测字节数必须超过配置阈值。

OOM 告警必须为 `critical`，包含真实 victim PID 和 `python3` 命令名，归因为 `monitored`，并携带命名根进程的稳定谱系标记。

完整告警验收后，场景会停止 gateway。

断连期间继续跨越多个资源采样周期，要求 daemon 保持运行，
并且独立 Evidence SQLite 与 Sandbox Alert SQLite 均无新增记录。

gateway 重新启动后，场景再次执行真实 `actrail-sb connect`。

重连后必须收到新采样的资源告警，
且不能补发断连期间产生的旧观测。

该 case 的验收边界是 Host native-VSOCK 组件数据通路、Web 在线阈值配置和上述四类告警。
Firecracker VMM transport 与快照恢复后的 Guest eBPF link 延续性由 Firecracker 主线场景验收。

仓库根目录运行：

```bash
PYTHONDONTWRITEBYTECODE=1 \
deploy/virtual-container/host/run-v2-tests.sh \
  --no-profile \
  --case sandbox_resource_alert_host \
  --color never \
  --fail-fast
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 Cargo、Rustup 和 PATH。
显式 `--no-profile` 表示该 Host case 不读取 Kata 本机 profile。
