# ADR 0003：无网络 Guest 使用 VSOCK bridge 出境

> 本文记录无网络 Guest 通过 VSOCK bridge 连接 Host collector 的架构决策。

Status: accepted

Scope: 部署 adapter

## 背景

没有 CNI（Container Network Interface）的 Kata Guest 没有可用外网，但 `actraild` 仍可能要向 Host collector 发送 OTLP/HTTP（OpenTelemetry Protocol over HTTP）。在 relay 中重新实现 acknowledgement、TLS、retry 和 batching 会产生第二套 delivery protocol。

## 决策

部署提供两种 egress mode：

- `network`：使用 Guest 网络和 node-local collector endpoint；
- `vsock-bridge`：使用 Guest loopback TCP-to-VSOCK bridge 与 Host VSOCK/Unix-to-loopback TCP bridge。

bridge 是由 systemd 管理的前台 `socat` socket 转发进程。它只复制字节，不终止 TLS、不解析 OTLP、不持久化、不确认、不重放。Host 侧只能连接固定 loopback collector endpoint。Firecracker/StratoVirt 使用 AF_VSOCK；Cloud Hypervisor 使用 per-VM Unix endpoint 和生命周期 reconcile。

bridge 位于 exporter connection 下方，承担部署层的字节转发。

## 后果

- OTLP batching、TLS 校验、有界 retry、response 分类和 shutdown flush 由 `actraild` 与 collector 端到端完成。
- systemd 负责 restart、日志、limit 和 process-group 清理。
- bridge 不提供 store-and-forward；中断超过 exporter retry budget 时，按 exporter delivery contract 显式丢弃数据。
- 引入 native VSOCK dialer 必须先由性能测量证明其必要性。
