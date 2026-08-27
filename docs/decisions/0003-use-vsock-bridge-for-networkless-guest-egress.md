# ADR 0003：无网络 Guest 使用 VSOCK bridge 出境

> 本文记录无网络 Guest 采用 VSOCK bridge 出境并保持现有 exporter delivery contract 的架构决策。

Status: accepted

Scope: 初始部署 adapter

## 背景

没有 CNI（Container Network Interface）的 Kata Guest 没有可用外网，但 `actraild` 仍可能要向 Host collector 发送 OTLP/HTTP（OpenTelemetry Protocol over HTTP）。在 relay 中重新实现 acknowledgement、TLS、retry 和 batching 会产生第二套 delivery protocol。

## 决策

保留两种部署期 egress mode：

- `network`：使用 Guest 网络和 node-local collector endpoint；
- `vsock-bridge`：使用 Guest loopback TCP-to-VSOCK bridge 与 Host VSOCK/Unix-to-loopback TCP bridge。

初始 bridge 是由 systemd 管理的前台 `socat` socket 转发进程。它只复制字节，不终止 TLS、不解析 OTLP、不持久化、不确认、不重放。Host 侧只能连接固定 loopback collector endpoint。Firecracker/StratoVirt 使用 AF_VSOCK；Cloud Hypervisor 使用 per-VM Unix endpoint 和生命周期 reconcile。

它是现有 exporter connection 下方的 deployment adapter，不改变 `actraild` exporter 代码或运行时配置语义。

## 后果

- OTLP batching、TLS 校验、有界 retry、response 分类和 shutdown flush 仍端到端存在于 `actraild` 与 collector 之间。
- systemd 负责 restart、日志、limit 和 process-group 清理，无需自建 PID/state lifecycle。
- bridge 不提供 store-and-forward；超过 exporter retry budget 的中断仍按既有 contract 显式丢失数据。
- native VSOCK dialer 只在测量证明需要时作为后续优化。
