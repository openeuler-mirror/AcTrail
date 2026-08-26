# 插件规范

> 本文汇总维护者实现或审查插件生命周期、故障隔离和 OTEL 出境行为所需的规范。

本目录记录实现必须保持的插件行为和安全边界。操作入口见
[插件操作](../../operations/plugins/README.md)，函数和数据格式见
[插件 API 参考](../../reference/plugin-api/README.md)。

- [生命周期与隔离](lifecycle-and-isolation.md)
- [OTEL exporter](otel-exporters.md)
