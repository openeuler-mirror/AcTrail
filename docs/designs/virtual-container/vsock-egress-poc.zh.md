# Kata Guest 无网络 OTLP 出境方案（POC）

## 背景与目标

Kata Guest 的网络来自 sandbox netns：CNI 在 netns 里配好网卡后，Kata shim 才把它
映射成 Guest 的 `eth0`。没有 CNI 时（例如 `ctr run` 直接起 sandbox），Guest 内只有
`lo`、没有路由，Guest 内的 `actraild` 无法把 OTLP/HTTP trace 送到 Host 上的
Collector。

本阶段只解决实时数据通道：

```text
actraild
  -> Guest bridge（loopback TCP -> VSOCK）
  -> Host bridge（VSOCK/UDS -> loopback TCP）
  -> Host Collector（127.0.0.1:4318）
```

不在本阶段增加持久化队列、应用层 Relay、通用 Guest 网络或生产级多租户管理。

## 定位：这是双轨中的一轨

Guest 出境有两条路，产品两者都要支持，因此本方案不是"唯一通道"而是 `network`
轨的对偶：

- `network` 轨：Kubernetes/CNI 提供 Guest 网络，endpoint 指向 node-local
  Collector。此时**零新增组件**，现有部署工具链直接可用。
- `vsock-bridge` 轨（本文档）：没有 CNI 时使用，不触碰宿主网络栈。

两轨的唯一分歧点是"Guest 里 endpoint 指向哪、那个地址背后由谁承载"。上层的
exporter、TLS、批处理、有界重试、shutdown flush 完全不变——本方案一行 `actraild`
代码都不改即可工作，正是这一点的证据。因此模式是**部署期**维度
（`--egress-mode`），不进入 daemon 运行期。

## 决策

采用 **systemd 管理的前台 `socat` bridge** 作为最小验证和初始部署方案：

- Guest bridge 对两种 VMM 使用同一个接口：连接 Host CID `2` 的专用 VSOCK
  端口；
- StratoVirt Host adapter 监听 `AF_VSOCK`；
- Cloud Hypervisor Host adapter 监听该 VM VSOCK UDS 的端口后缀，并由 reconcile 随
  sandbox 生命周期起停（见下）；
- Host bridge 只能连接 `127.0.0.1:4318`，不能由 Guest 选择目的地址；
- bridge 只双向复制字节，不终止 TLS、不解析 OTLP、不缓存或重放数据；
- 默认走明文 HTTP，HTTPS 作为可选加固档（见"明文与 TLS 的档位"）。

bridge 自身不实现 `start/status/stop`、PID 文件或 state directory。它以前台进程
运行，systemd 负责启动、重启、日志、资源约束和停止。

## 为什么是 VSOCK 通道

先于"谁来搬字节"的问题，是"走哪条通道出 Guest"。候选与结论：

| 通道 | 依赖 | 改核心代码 | 结论 |
| --- | --- | --- | --- |
| CNI/netns 给 Guest 网络 | 宿主网络栈与防火墙，CNI ADD/DEL 生命周期 | 无 | K8s 下天然满足，即 `network` 轨；裸机下需自建并精确撤销宿主网络资源 |
| **VSOCK bridge** | Guest/Host 各一个 `socat` | 无 | **不碰宿主网络栈、可精确撤销，选为无 CNI 场景的通道** |
| `actraild` 原生 VSOCK | 无外部进程 | 有 | 需泛型化现有 `TcpStream` 连接 seam，见下节 |
| virtio-fs 落盘 + Host 侧读取 | 额外共享挂载 | 无 | 落点虽在 Host 上，但要新开一条 file→Collector 管线并处理轮转/去重/fsync |
| kata-agent 既有通道、debug console | 无 | 无 | 协议固定，不允许旁路业务数据 |
| 不实时出境、销毁前取回 SQLite | 无 | 无 | Guest 随 sandbox 销毁，本地文件不可靠 |

## Cloud Hypervisor 的 sandbox 生命周期

Cloud Hypervisor 使用 hybrid VSOCK：Guest 连接 CID 2 的端口，VMM 把它转发到 Host 上
的 `<VM base UDS>_<port>`。base UDS 位于 Kata 自行创建和删除的 per-sandbox 目录，
因此它无法像 StratoVirt 那样用单一常驻 listener，也无法为尚不存在的 sandbox 预先
写配置。

两条实测事实决定了最终形态：

1. base socket 路径是 `/run/vc/vm/<sandbox>/clh.sock`，sandbox 目录名即 sandbox id
   ——因此模板 unit 用 `%I` 即可推导，**不需要任何人工编写的 per-sandbox 配置**；
2. bridge 在 sandbox 起来**之后**建立仍然有效，因为 Guest exporter 会重试——因此
   不必与 VM 启动抢跑，监听目录变化再 reconcile 就够。

于是：`.path` unit 监视 `/run/vc/vm`，触发 reconcile 为每个活 sandbox 启动实例、
停止 sandbox 已消失的实例（`socat` 会持有监听 socket 不自行退出）。sandbox id 经
`systemd-escape` 转义，`%I` 还原。

## 明文与 TLS 的档位

默认档是明文 HTTP。VSOCK 通道上的字节只经过 Host 的内核与内存，不进入任何网络
接口，所以默认档省掉 CA 生成与分发进 Guest 镜像、证书 IP SAN 和轮换这一整套；
Host 上 bridge 到 Collector 也只经 `127.0.0.1`。

需要额外加固时切到 HTTPS：endpoint 用 `https://127.0.0.1:<port>/v1/traces`，
Collector 证书带 IP SAN `127.0.0.1`。exporter 对可解析为 IP 的 host 按 IP SAN 校验
（IPv6 字面量不支持）。两档都保持 `actraild` 与 Collector 端到端，bridge 只看密文。

## 为什么选择这个方案

### 1. 不改变既有投递语义

OTLP/HTTPS 批处理、TLS 校验、有界重试、HTTP 响应分类和关闭 flush 都已经固化在
`actraild` 的 `otel-http` exporter 中。把 VSOCK seam 放在 TLS 下方，意味着这些行为
全部保持原样，VMM 差异不会进入 exporter。

Host bridge 看到的只是 TLS 密文。Collector 的 HTTP 状态码、`Retry-After` 和部分成功
响应会通过同一条通道原样返回给 `actraild`，不需要第二套确认或重试协议。

### 2. 运行开销小且可预期

正常情况下 `actraild` 会复用一条 HTTP keep-alive 连接，因此 bridge 只维持少量
长期字节流，不进行序列化、压缩、存储或业务处理。它的开销主要是进程本身和内核
socket buffer，相比 Guest、`actraild` 和 Collector 的整体开销较小。

本方案不引入磁盘 spool、消息队列或第二个 OTLP pipeline，也就不会增加相应的
磁盘 I/O、数据副本和状态协调成本。

### 3. systemd 提供足够的进程可靠性

手工运行前台 bridge 虽然代码最少，但退出后不会自行恢复。systemd 可以直接管理
同一个前台进程，并提供：

- `Restart=on-failure` 自动恢复；
- journal 日志；
- `MemoryMax`、`TasksMax` 和文件描述符限制；
- 统一的进程组停止，避免残留子进程；
- 明确的启动和关闭顺序。

Guest bridge 在 `actraild` 之前启动。systemd 关闭时按相反顺序执行，使
`actraild` 先完成有界 flush，再停止 bridge，避免提前拆掉最后一段出境通道。

### 4. 不重复实现操作系统已有的生命周期能力

自建 `start/status/stop` 需要保存 PID、检查 PID 是否复用、维护 state directory、
处理异常退出和清理残留状态。这些代码与数据转发无关，而且容易形成新的故障点。

让 bridge 保持前台、把生命周期交给 systemd，可以删除整套自定义进程管理接口。
Cloud Hypervisor adapter 只处理自己创建的精确 UDS 端口后缀，不做目录级或模糊
清理。

Cloud Hypervisor unit 使用 `ProtectSystem=full` 而不是 `strict`：后者会把 VMM 管理的
runtime 目录一并设为只读，使 bridge 无法在 base UDS 旁创建端口后缀；`full` 仍将
`/usr`、`/boot` 和 `/etc` 设为只读。该 unit 也不启用 `PrivateTmp`，因为 Cloud
Hypervisor 官方支持把 base UDS 放在 `/tmp`；私有 `/tmp` 会让 VMM 看不到 bridge
创建的端口后缀。

### 5. 对现有代码侵入较小，同时保留升级路径

该方案只增加部署 adapter，不需要修改 `live_http.rs`、OTLP payload、TLS 配置模型
或 Kata agent 协议。撤销这些部署文件即可回到原有普通网络出口。

如果后续实测证明 bridge 的进程数或转发开销成为瓶颈，可以在保持上层接口不变的
情况下，把 Guest bridge 替换为 `actraild` 内部的原生 VSOCK `StreamDialer`，并把
Host bridge 替换为单进程异步实现。本阶段不提前承担这部分核心代码改动。

## 为什么暂不选择其他方案

以下比较的是**选定 VSOCK 通道之后**由谁来搬字节；通道本身的取舍见"为什么是 VSOCK
通道"。

### 手工运行前台 bridge

代码最少，但进程退出后没有自动恢复，启动和关闭顺序依赖人工操作。适合一次性命令
验证，不适合作为初始部署方式。

### 自定义 PID/state 管理脚本

可以提供 `start/status/stop`，但重复实现 systemd 已有能力，增加 PID 复用、状态残留
和清理正确性的维护成本，对数据通道本身没有额外价值。

### `actraild` 原生 VSOCK

这是长期运行开销更低的 Guest 侧方案，因为可以删除 Guest bridge；但它需要在
现有 HTTPS exporter 下方引入新的字节流 adapter，并修改、回归核心连接代码。当前
先用外部 bridge 验证 VSOCK 与双 VMM 路径，再决定是否承担该改动。

### Collector 原生 VSOCK receiver

理论上可以同时删除 Host bridge，但需要维护定制 Collector 构建，并处理
StratoVirt `AF_VSOCK` 与 Cloud Hypervisor per-VM UDS 两种 listener。相对当前只连接
Host Collector 的目标，维护成本过高。

### Host 终止 OTLP/TLS 或增加应用层 Relay

这会引入第二个 OTLP pipeline、重试状态和投递确认 seam，并改变 `otel-http` exporter 已确认
的 TLS 与投递责任，不属于单纯的数据通道转换。

### virtio-fs 持久化 spool

spool 能提高长时间断连时的数据保留能力，但必须新增文件所有权、`fsync`、轮转、
确认、重放和 Guest 销毁语义。当前投递合同是有限重试和明确丢弃，不要求持久化
store-and-forward，因此不引入该复杂度。

## 可靠性边界

systemd 解决的是 bridge 进程退出后的恢复，不改变 `otel-http` exporter 的数据可靠性合同：

- 短暂中断：bridge 恢复后，`actraild` 按既有策略重试；
- 中断超过重试预算：该批数据被明确记录为丢弃；
- bridge 不缓存、不主动重放，不把失败误报为 Collector 已接收；
- 若未来要求长期断连不丢数据，必须单独设计持久化 spool 和去重/确认协议。

## 生命周期顺序

启动：

1. 启动支持 HTTPS 的 Host Collector；
2. 启动对应 VMM 的 Host bridge；
3. 启动 Kata sandbox 和 Guest bridge；
4. 启动 `actraild`，由其建立并复用 HTTPS 连接。

关闭：

1. 停止产生新 trace；
2. 停止 `actraild`，等待 exporter 的有界 flush 完成或超时；
3. 停止 Guest bridge；
4. 停止 Host bridge；
5. 关闭 sandbox/VM 和 Host Collector。

## 当前范围与后续升级条件

范围是 bridge、systemd unit、CH 的 sandbox reconcile、合同测试、Host Collector TLS
示例，以及部署脚本中的 `--egress-mode` 维度。**`actraild`、Kata agent 和现有
Collector 默认配置均未修改**——这也是"两轨只差一个 endpoint"这一判断的直接证据。

满足以下任一条件后，再讨论原生 VSOCK 实现：

- bridge 的进程或内存开销经测量成为实际瓶颈；
- 需要大量 sandbox 并发连接和统一配额；
- 需要结构化连接指标和更细的 sandbox 归属；
- 项目允许修改并完整回归 `live_http.rs` 的连接 seam。

## 已知边界

- **StratoVirt 的 Host listener 绑定 `VMADDR_CID_ANY`**：节点上任意 Guest 都能连到
  该端口。单租户验收可接受；多租户需按 VM 分端口或由 Collector 认证发送方。Cloud
  Hypervisor 因每 sandbox 一个 UDS 而不存在此问题。
- Kata 默认 Guest 内核不带 BTF，`actraild` 会 `ebpf auto-degraded` 并按两轴 `auto`
  降级。这不影响出境通道，但全量 eBPF 采集需要带 BTF 的 Guest 内核。

## 参考合同

- [Cloud Hypervisor v51.1 VSOCK 文档](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v51.1/docs/vsock.md)：
  Guest 连接 Host 时使用 CID `2`，Host listener 路径为 VMM base UDS 加
  `_<port>` 后缀；
- [socat 1.8 手册](https://manpages.debian.org/trixie/socat/socat.1.en.html)：
  `VSOCK-CONNECT`、`VSOCK-LISTEN`、`UNIX-LISTEN` 与 `unlink-close` 的接口合同。
- 实测（Kata 3.32）：Cloud Hypervisor 的 base UDS 为
  `/run/vc/vm/<sandbox>/clh.sock`（无中间 `root/` 层级）；VSOCK 端口 1024/1025 用于
  kata-agent，1026 为 debug console，故本方案的端口下限取 1027。
