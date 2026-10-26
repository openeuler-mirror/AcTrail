状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config/src/daemon/finalization/ # 配置观测结束后的任务调度与收尾预算。
└── [x] config.rs # 定义观测后任务的容量、超时参数和默认常量。
    └── [x] struct PostTraceRuntimeConfig # 限制并行任务、代理队列、每轮请求及准入、执行、关闭等待时间。
```
