# Sandbox Resource Alert Host 回归

## 测试目标

验证 Host native VSOCK 的资源观测、告警存储和公开订阅投递，并验证在线阈值更新与断连恢复行为。

## 场景设计

在命名根进程下运行真实 xiaoO 和文件工作负载，以受控 memory cgroup 产生 OOM。采集 daemon 显式连接 gateway 后验证四类告警，通过 Web 更新内存阈值，再断开和重建连接检查运行状态及新观测。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储与查询 |
| 主机 | Linux、root、native AF_VSOCK loopback、可用 memory cgroup | 传输资源观测并执行受控 OOM |
| 运行组件 | actrail-sb、VSOCK gateway、alert proxy、actrailweb | 采集、转发与在线配置 |
| xiaoO | 真实 CLI 可用 | 执行 Bash tool call 和文件任务 |

## 测试步骤摘要

1. 刷新 actrail-sb 默认配置，启动采集 daemon 并等待 control socket。
2. 在尚未连接时跨越多个采样周期，核对存活与数据库。
3. 启动 Host gateway，显式执行 connect 后启动工作负载。
4. 通过 Web 降低再提高内存风险阈值，核对同一插件实例产生新告警。
5. 由真实 xiaoO 触发专属 cgroup 内的受控 OOM，并完成文件读写。
6. 核对独立告警库与 subscriber 的四类告警。
7. 停止 gateway 并持续采样，再重启并显式连接，核对恢复结果。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 未连接与断连 | 跨越多个采样周期 | daemon 存活，Evidence 和 Alert SQLite 无新增记录 |
| 在线配置 | Web 更新阈值并写回原配置 | 同一插件实例按新阈值产生内存风险告警 |
| 告警完整性 | 工作负载完成 | 数据库和 subscriber 同时出现内存风险、OOM killed、高读取量和高写入量告警 |
| 读写归属 | 命名根进程的子进程执行读写 | PID、启动时钟、进程名匹配，字节数超过阈值 |
| OOM 归属 | 32 MiB cgroup 内 allocator 被内核杀死 | critical 告警带真实 victim PID、python3、monitored 归因和稳定谱系标记 |
| 重连 | gateway 重启并显式连接 | 收到新采样告警，断连期间旧观测不补发 |

## 快速运行

仓库根目录运行：

```bash
PYTHONDONTWRITEBYTECODE=1 \
deploy/virtual-container/host/run-v2-tests.sh \
  --case sandbox_resource_alert_host \
  --color never \
  --fail-fast
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 Cargo、Rustup 和 PATH。
该 Host case 直接使用运行环境，不需要本机 profile。

## 手动测试

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
工具命令通过位置参数传入 Python 触发脚本路径和测试协调目录，参数在生成命令时进行 shell 转义。
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
