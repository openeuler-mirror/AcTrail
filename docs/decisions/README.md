# 架构决策

> 本文说明长期架构决策的背景、结果及其必须保留的后果。

ADR（Architecture Decision Record）记录长期技术选择及其后果。当前系统结构见[架构](../architecture/README.md)，规范性行为见[规范](../specifications/README.md)。

文件名采用 `NNNN-short-title.md`，状态为 proposed、accepted、superseded 或 rejected。

- [隔离手侧观测通路](0001-isolate-hand-observation-path.md)
- [使用 pidfd 完成 launch 注册](0002-use-pidfd-for-launch-registration.md)
- [无网络 Guest 使用 VSOCK bridge 出境](0003-use-vsock-bridge-for-networkless-guest-egress.md)
- [使用独立告警代理](0004-use-a-standalone-alert-proxy.md)
