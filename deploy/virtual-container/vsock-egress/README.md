# Kata Guest 无网络 OTLP 出境（VSOCK bridge）

> 完整部署步骤以 [虚机容器部署](../../../docs/operations/deployment/virtual-container.md) 为准；本页说明该组件的原理、参数和边界。

Kata Guest 只有在 CNI/Kubernetes 为 sandbox 配好网络时才有 `eth0` 和路由。没有
CNI 时（例如 `ctr run` 直接起 sandbox），Guest 内只有 `lo`，`actraild` 的
OTLP/HTTP 发不出去。

本目录提供 VSOCK bridge，让这种 Guest 把 trace 送到同一台 Host 上的 Collector，
不依赖任何宿主网络配置，撤销时只需删除这些文件。

```text
actraild
  -> http://127.0.0.1:14318/v1/traces      Guest loopback
  -> Guest bridge                          loopback TCP -> AF_VSOCK CID 2:43180
  -> Host bridge                           StratoVirt: AF_VSOCK
                                           Cloud Hypervisor: <clh.sock>_43180 UDS
  -> 127.0.0.1:4318                        Host Collector
```

bridge 只复制字节，不解析 OTLP、不终止 TLS、不缓存、不重放。Host 侧目的地固定为
`127.0.0.1`，Guest 不能把它变成通用代理。

这里的“OTLP 出境”只描述传输路径，不扩大数据授权。只有显式传入
`--otel-endpoint` 才会安装并加载 `otel-http`；其配置默认使用
`attribute_mode = "metadata-only"`，因此只发送 semantic action/trace 的
结构化元数据，不发送命令行和 HTTP/LLM 内容属性。可信 Collector 场景可在生成部署包时
传入 `--otel-attribute-mode full`；这会允许
semantic action 内容属性出境，但不会把 SQLite 中的原始 `payload_segments` 字节
自动编码进 OTLP span。

## 这是两种出境模式之一

默认部署不传 endpoint，仅使用 Guest 本地 SQLite。启用 OTLP 后，出境是一个
**部署期**维度，由 `--egress-mode` 选择，`actraild` 本身不区分：

| 模式 | endpoint | 背后是谁 | 适用 |
| --- | --- | --- | --- |
| `network`（默认） | `https://<node-ip>:4318/v1/traces` | CNI 给的 Guest 网络 → node-local Collector | Kubernetes |
| `vsock-bridge` | `http://127.0.0.1:14318/v1/traces` | 本目录的 bridge | 无 CNI 的裸机/验收环境 |

两种模式共用同一份基础镜像：`socat` 总是安装，只有 bridge unit 的启用与否随模式
变化。校验规则按模式分派，`network` 模式下 Guest loopback 仍然被拒绝。

## 已验证状态

真机验证（Kata 3.32，Host 需加载 `vhost_vsock`）：

- **StratoVirt**：真 vhost-vsock，Host 节点级 `AF_VSOCK` listener；
- **Cloud Hypervisor**：hybrid vsock，Host 按 sandbox 使用 `<clh.sock>_43180` UDS，
  bridge 由 reconcile 自动随 sandbox 起停；
- Guest 镜像由 `guest/build-openeuler-image.sh` 构建（openEuler 24.03），
  prepare 通过 `guest/inject-image.sh` 向副本注入 AcTrail 和出境配置。
- x86_64 openEuler 24.03（LTS-SP1）Host 上，Cloud Hypervisor 51.1 同样通过完整
  interface/data 矩阵；两个并发 sandbox 各自持有独立 bridge 实例，全部销毁后既无
  `actrail-vsock-host-cloud-hypervisor@` 实例也无 `clh.sock_43180` 残留。

对照实验：同一个 sandbox 内 `ip addr` 只有 `lo`、无路由、够不到宿主任何地址，而经
bridge 的 `POST /v1/traces` 到达 Host Collector 并返回 200。

## 前提

- Host 加载 `vhost_vsock`（StratoVirt 必需）；
- Host 安装 `socat >= 1.7.4`，构建时启用 `VSOCK-CONNECT`/`VSOCK-LISTEN`；
- Guest 镜像内含 `socat`（见 `guest/build-openeuler-image.sh` 的包列表）；
- Host Collector 监听 `127.0.0.1:4318`。容器化 Collector 需使用 host 网络。

专用 VSOCK 端口默认 `43180`，可在 1027-65535 内修改：生成部署包时传
`prepare-artifacts.py --vsock-port PORT`（写入 Guest bridge unit 和 `manifest.json` 的
`vsock_port`），Host bridge 用 systemd drop-in 传同一个 `--vsock-port`（见部署文档 5.1）。
`1-1026` 被拒绝，避开 Kata agent 通信、日志和调试端口（其中 1026 是 debug console）。

## Guest 侧：由镜像构建/注入完成

Guest bridge 不需要手工安装。构建或注入镜像时选择模式即可：

V2 虚机容器验收优先走标准 artifact 准备器，它会注入 Guest 镜像（base/data 同源时共用一张）、生成
runtime config 和本机 profile：

```bash
OPEN_EULER_2403_IMAGE=/path/to/openeuler-24.03-kata.image

sudo -E env "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-artifacts.py \
    --backend cloud-hypervisor \
    --otel-endpoint http://127.0.0.1:14318/v1/traces \
    --egress-mode vsock-bridge \
    --base-config-source /path/to/configuration-clh.toml \
    --data-config-source /path/to/configuration-clh.toml \
    --base-image-source "$OPEN_EULER_2403_IMAGE" \
    --data-image-source "$OPEN_EULER_2403_IMAGE" \
    --data-kernel /path/to/vmlinux-debug.container \
    --workload-image docker.io/library/actrail-openeuler-workload:24.03 \
    --xiaoo /path/to/xiaoo
```

上例显式固定 Guest 与 workload 为 openEuler 24.03。StratoVirt 可省略 `--backend`，并换成对应的 base/data source config；
同样传入这三个显式镜像参数即可验证相同组合。模式会写入内容寻址输入和
`manifest.json`；同一 source image 的 `network` 与 `vsock-bridge` 产物不会错误命中
同一个缓存。

下面两个低层工具用于独立制作或调试单张镜像：

```bash
# 构建干净的 openEuler Guest 镜像（不含 AcTrail）
guest/build-openeuler-image.sh \
  --rootfs /var/tmp/oe-rootfs \
  --output-image /opt/actrail-images/openeuler-24.03-guest.image \
  --kata-initrd /opt/kata/share/kata-containers/kata-containers-initrd.img \
  --expected-agent-version 3.32.0

# 向该镜像的副本注入 AcTrail（prepare 内部同样调用它）
guest/inject-image.sh \
  --source-image /opt/actrail-images/openeuler-24.03-guest.image \
  --output-image actrail-guest.image \
  --bundle /var/lib/actrail/guest-bundle \
  --otel-endpoint http://127.0.0.1:14318/v1/traces \
  --egress-mode vsock-bridge
```

`--kata-initrd` 必须来自**运行该镜像的** Kata 版本，Guest 内的 kata-agent 要与之
匹配，而不是构建机上恰好装着的版本。

`vsock-bridge` 模式下安装器会：

- 校验 endpoint 是 `127.0.0.0/8` 字面量且带显式端口（`localhost` 被拒绝：它取决于
  Guest 的 DNS 与 `/etc/hosts`）；
- 用该端口渲染 bridge unit 的 `ExecStart --listen-port`，**endpoint 是端口的唯一
  真源**，exporter 与 bridge 不可能配不一致；
- 启用 bridge unit；`network` 模式下则确保镜像中不残留 bridge。

安装结果记录在 `/usr/share/actrail/guest-install-info` 的 `guest_egress_mode`，
`guest/verify-rootfs.sh` 会离线断言。

## Host 侧安装

安装与启用步骤（含 Collector 端口覆盖配置）见
[虚机容器部署 5.1](../../../docs/operations/deployment/virtual-container.md#51-安装宿主机转发服务)。本节只说明两种后端的转发机制。

### StratoVirt

一个节点级 listener 服务全部 sandbox：`actrail-vsock-host-stratovirt.service` 常驻监听 `AF_VSOCK` 端口 43180。

### Cloud Hypervisor

hybrid vsock 的 Host 端点是 `<VM base UDS>_<port>`，而 base UDS 位于 Kata 自行创建
和删除的 per-sandbox 目录 `/run/vc/vm/<sandbox>/clh.sock`，所以不能用单一常驻
listener，也无法为尚不存在的 sandbox 预先配置。

因此启用的是 `actrail-vsock-host-cloud-hypervisor-reconcile.path`，它随 sandbox 出现/消失自动起停 bridge。

`.path` unit 监视 `/run/vc/vm`，触发 `ch-reconcile.sh`：为每个暴露 `clh.sock` 的
sandbox 启动 `actrail-vsock-host-cloud-hypervisor@<sandbox>.service`，并停止 sandbox
已消失的实例。每个 bridge 也会监视自己的 base UDS；Kata 删除该路径时立即退出，
避免 systemd reconcile 繁忙时短暂超出 sandbox 生命周期。reconcile 仍是节点级真源
和恢复兜底。sandbox id 经 `systemd-escape` 转义，unit 中的 `%I` 还原。

bridge **不需要**先于 VM 存在：Guest 的 exporter 会重试，因此在 sandbox 出现之后
再建立通道即可。

## 明文与 TLS

默认档为明文 HTTP（`allow_insecure = true`，模板已如此）。字节全程只经过 Host 的
内核与内存，不进入任何网络接口，因此不引入 CA 分发、IP SAN 证书和轮换。

需要额外加固时改用 HTTPS：endpoint 写 `https://127.0.0.1:14318/v1/traces`，Collector
证书需包含 IP SAN `127.0.0.1`，Guest 配置 `tls_ca_cert_path`。exporter 对 IPv4 字面量
按 IP SAN 校验（IPv6 字面量不支持）。TLS 握手在 `actraild` 与 Collector 之间端到端
完成，bridge 只看到密文。

参考配置见 [`collector/otel-collector-tls.yaml`](collector/otel-collector-tls.yaml)：

```bash
export ACTRAIL_OTELCOL_TLS_CERT=/etc/actrail/tls/collector.crt
export ACTRAIL_OTELCOL_TLS_KEY=/etc/actrail/tls/collector.key
otelcol-contrib --config ./collector/otel-collector-tls.yaml
```

## 安全边界

- **StratoVirt 的 Host listener 绑定 `VMADDR_CID_ANY`**：该节点上任意 Guest 都能连接
  到这个端口，进而访问 Host Collector。单租户验收环境可接受；多租户场景必须按 VM
  分端口，或由 Collector 侧认证发送方。Cloud Hypervisor 无此问题——每个 sandbox 只
  能看到自己的 UDS。
- Host bridge 的目的地固定为 `127.0.0.1:<collector port>`，Guest 无法选择。
- Cloud Hypervisor bridge 只创建自己的 `<base>_<port>` 后缀；后缀已存在时拒绝启动且
  不删除它，正常退出时 `unlink-close` 也只处理该后缀，不触碰 VM base socket 或目录。

## 可靠性边界

systemd 解决的是 bridge 进程退出后的恢复，不改变既有投递合同：

- 短暂中断：bridge 恢复后 `actraild` 按既有策略重试；
- 中断超过重试预算：该批数据被明确记录为丢弃；
- bridge 不缓存、不重放，不把失败误报为已接收；
- Guest 随 sandbox 销毁，本地文件不可靠，因此生产长期留存应启用实时出境；
- 长期断连不丢数据需要单独设计持久化 spool 与去重/确认协议，当前不提供。

启动顺序为 Collector → Host bridge/reconcile → sandbox 与 Guest bridge →
`actraild`；关闭时反向，使 `actraild` 先完成有界 flush 再拆除通道。

## 运行与诊断

Host：

```bash
systemctl status actrail-vsock-host-stratovirt.service
systemctl list-units 'actrail-vsock-host-cloud-hypervisor@*.service'
journalctl -u actrail-vsock-host-cloud-hypervisor-reconcile.service
ls /run/vc/vm/<sandbox>/            # 应出现 clh.sock_43180
```

Guest（需在 Kata 配置中开启 `debug_console_enabled`，CH 下连接 sandbox base UDS 并
发送 `CONNECT 1026`）：

```bash
systemctl status actraild.service actrail-vsock-guest-bridge.service
ss -ltn                              # 应出现 LISTEN 127.0.0.1:14318
grep endpoint /etc/actrail/plugins/otel-http/otel-http.config.toml
```

## 合同测试

不需要 VSOCK 设备或 root，使用 fake `socat`/`systemctl` 检查公开命令与 unit 合同：

```bash
tests/e2e/regression/virtual_container/test-vsock-egress-contract.sh
```

该脚本就是 V2 回归执行的唯一一份 VSOCK 部署断言；运行时证据写入 V2 runner 分配的
case workspace，由 runner 的 `--cleanup/--no-cleanup` 统一决定是否保留。

合同测试通过不等于完成真机验证：真机需确认 Guest 无路由时生成的 trace 出现在 Host
Collector 中。设计取舍与原生 VSOCK 的升级条件见
[`../../../docs/decisions/0003-use-vsock-bridge-for-networkless-guest-egress.md`](../../../docs/decisions/0003-use-vsock-bridge-for-networkless-guest-egress.md)。
