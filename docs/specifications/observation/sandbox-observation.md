# 沙箱观测

> 本文定义 Guest 手侧观测模型的信号范围与路由边界。

Status: Implemented
Owner: 执行隔离观测通路
Scope: Guest-local 观测语义与 daemon 路由

## 模型边界

手侧模型只包含：

- 每个 root lineage 的 read/write 操作次数和字节数增量，包括失败次数；
- 整个 Guest 的 CPU 与内存累计快照；
- 带三态归因和可选 monitored root 的 OOM victim 事件。

它不得包含文件或网络内容、syscall payload、脑侧进程身份、trace ID 或 semantic action。**Root lineage** 是匹配到的根进程及其通过 fork、vfork 或 clone 创建的后代；在一次 Guest boot 内由根 PID、启动时间和发现时匹配的 Linux `comm` 名共同标识。后代在 exec 后仍属原 lineage，进程退出后移除；PID 复用不得把观测错误归到旧根进程。

成功的 `read` 和 `write` 增加操作次数及实际返回字节数；负返回值只增加对应失败次数。内核采集必须聚合计数，不得复制用户缓冲区、计算内容哈希或逐 syscall 发送事件。

Guest 资源快照不依赖目标进程是否存在。CPU consumer 通过相邻累计快照计算区间利用率。`vmstat` 的 `oom_kill` 只是累计资源指标，本身不生成 `OomKilled`；具体 victim 由 OOM tracepoint observation 表达。

## 有界发布

所有 queue、batch、map、event buffer 和保留的来源状态都必须具有已校验的容量。慢速 sender 或 consumer 不得阻塞采集。queue 满时记录显式 drop；OOM queue 满只增加 drop 诊断，不新增常驻 worker。

只有当前有效的 VSOCK session 才可把 observation 放入发送 queue。断连期间的 observation 不缓存、不持久化、不重放，但 Process I/O baseline 仍持续推进。

## 路由

每条 observation 的成功 interest query 只能选择一种路由：

- 存在一个或多个匹配插件：投递给全部匹配项；
- 没有匹配插件：写入独立的 Sandbox Evidence DB。

Interest query、插件投递或 evidence store 的错误必须保持为该操作自身的失败，不得转换成另一条路由，也不得进入 AcTrail 主存储。
