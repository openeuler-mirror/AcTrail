# 插件操作

> 本文为插件管理员和插件作者提供插件启用、更新、排错与接口文档入口。

插件管理员按 [管理插件](manage.md) 建立统一的生命周期和权限模型，再进入具体插件的
启用页。插件作者从 [插件 API 参考](../../reference/plugin-api/README.md) 开始。

## 操作入口

1. 插件管理员通过 [管理插件](manage.md) 了解安装、发现、加载、授权和持久化。
2. 插件管理员按插件类型选择正式启用路径：
   - [activity-anomaly](activity-anomaly.md)：请求/响应增长和长命令告警。
   - [otel-http](otel-http.md)：向 OTLP/HTTP Collector 实时发送 span。
   - [otel-jsonl](otel-jsonl.md)：写入本地 OTLP JSONL，或通过 JSON-RPC HTTP(S) 交付。

插件包中的 manifest、配置和 schema 是部署资产。安装插件包不会授予权限，也不会
启动插件；只有显式加载或启动清单才会创建运行实例。

这里的 **manifest** 是描述插件身份、角色、运行时、资源限制和所需权限的 TOML 文件；
**schema** 是加载时校验插件业务配置的 JSON Schema。
