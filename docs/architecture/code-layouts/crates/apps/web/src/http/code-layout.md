状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/web/src/http/ # 校验请求参数，将 HTTP 路由交给对应数据查询或管理操作。
├── [x] resources.rs # 提供独立资源查询 HTTP 接口，按 operator 配置目录解析资源数据库。
│   └── [x] struct ResourceHttp # 借用 WebContext，解析资源数据库并响应宿主和工作负载查询。
├── [x] plugins.rs # 处理插件目录、实例、命令和配置接口。
│   └── [x] struct PluginHttp # 借用 WebContext，将已校验的管理操作交给 daemon 控制入口。
├── [x] alerts.rs # 处理告警查询、订阅相关请求。
│   └── [x] struct AlertHttp # 借用上下文中的告警数据源生成响应。
├── [x] checkpoints/ # checkpoint 历史与 operation 的 HTTP 接口；集群模式和未启用 checkpoint 时不可用。
│   ├── [x] mod.rs # 分派 checkpoint 路由，每个请求以只读方式打开 operator 配置中的记录库；其他 Trace 的 checkpoint 和 operation 编号按不存在处理。
│   │   └── [x] struct CheckpointHttp # 借用 WebContext，读取 checkpoint 记录库并响应查询。
│   ├── [x] listing.rs # 返回 Trace 的 scope、checkpoint 历史与 operation，附带每个边界匹配到的 LLM 调用；另向 daemon 查询当前 held/busy，连接和读写分别限时 500 ms，查询失败时 runtime 为 null，历史照常返回。
│   ├── [x] navigation.rs # 取 operation 边界记录的 llm.request 在 ActionTree 中所在的 llm.call，两个方向共用；观测数据已清理时没有对应调用，历史照常返回。
│   └── [x] actions.rs # 把保存、恢复、强制执行、解除暂停和删除转为控制命令交给 daemon，由 daemon 按调用者授权，返回 operation 编号。
├── [x] trace_routes.rs # 分派单 trace 的详情、动作、负载和统计查询。
├── [x] cluster_routes.rs # 分派集群索引、关系图和 trace 查询。
├── [x] trace_list.rs # 校验 trace 列表过滤条件与分页上限。
│   └── [x] struct TraceListLimits # 保存默认与最大查询数量约束。
├── [x] timeline.rs # 校验时间窗口、动作与关系边查询数量。
│   └── [x] struct TimelineLimits # 限制单次时间线响应的数据量与扫描范围。
├── [x] idle.rs # 校验空闲区间查询条件与页大小。
│   └── [x] struct IdleLimits # 保存空闲区间分页查询限制。
├── [x] query.rs # 解析共用查询字符串、标识符和时间范围；查询值先将 + 解为空格再解百分号，保留 %2B 为加号，URL 路径解码不转换 +。
└── [x] query/ # 约束按需内容读取的粒度和响应预算。
    └── [x] content.rs # 校验 LLM 内容导航和分页参数。
        └── [x] struct ContentLimits # 保存单次内容读取的条目与字节预算。
```
