# 告警代理设计

- [告警转发组件设计](alert-forwarding.md)：组件职责、进程边界、配置、状态、告警来源、过滤、故障隔离和性能边界。
- [传输协议](wire-protocol.md)：`actraild` 到 proxy 的本机二进制协议，以及 proxy 到外部订阅者的 JSON 协议。
- [运行时序](runtime-sequence.md)：进程启动、代理拉起、连接门控、订阅、广播、断连和重新启用行为。
