状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/web/src/http/ # 校验请求参数，将 HTTP 路由交给对应数据查询或管理操作。
├── [x] resources.rs # 提供独立资源查询 HTTP 接口，按 operator 配置目录解析资源数据库。
│   └── [x] struct ResourceHttp # 借用 WebContext，解析资源数据库并响应宿主和工作负载查询。
├── [x] plugins.rs # 处理插件目录、实例、命令和配置接口。
│   └── [x] struct PluginHttp # 借用 WebContext，将已校验的管理操作交给 daemon 控制入口。
├── [x] alerts.rs # 处理告警查询、订阅相关请求。
│   └── [x] struct AlertHttp # 借用上下文中的告警数据源生成响应。
├── [x] trace_routes.rs # 分派单 trace 的详情、动作、负载和统计查询。
├── [x] cluster_routes.rs # 分派集群索引、关系图和 trace 查询。
├── [x] trace_list.rs # 校验 trace 列表过滤条件与分页上限。
│   └── [x] struct TraceListLimits # 保存默认与最大查询数量约束。
├── [x] timeline.rs # 校验时间窗口、动作与关系边查询数量。
│   └── [x] struct TimelineLimits # 限制单次时间线响应的数据量与扫描范围。
├── [x] idle.rs # 校验空闲区间查询条件与页大小。
│   └── [x] struct IdleLimits # 保存空闲区间分页查询限制。
├── [x] query.rs # 解析共用查询字符串、标识符和时间范围。
└── [x] query/ # 约束按需内容读取的粒度和响应预算。
    └── [x] content.rs # 校验 LLM 内容导航和分页参数。
        └── [x] struct ContentLimits # 保存单次内容读取的条目与字节预算。
```
