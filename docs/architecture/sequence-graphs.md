# 运行时序

本页说明跨组件的关键交互和状态交接。每条流程对应当前实现中的职责边界；静态关系见[架构图](c4-graphs.md)。

## 受控 Launch

ctl 准备启动目标和可选 TLS-sync 注入。默认 `bpf-copy` 由 daemon 在暂停屏障内生成并固定探针计划；TLS-sync 模式也向 daemon 请求计划解析。

```mermaid
sequenceDiagram
    actor User as 操作者
    participant CTL as actrailctl
    participant Child as launch child
    participant Daemon as actraild
    participant Runtime as 身份与 Trace Runtime
    participant Collect as 资源与采集服务
    User->>CTL: launch target
    CTL->>Daemon: 解析权限与 capture profile
    Daemon-->>CTL: 生效权限及采集需求
    CTL->>Child: clone 并原子取得 pidfd
    Child->>Child: 安装可选 seccomp，SIGSTOP
    CTL->>Child: waitpid 确认暂停
    CTL->>Daemon: TrackAdd + SCM_RIGHTS pidfd
    Daemon->>Daemon: 校验目标，准备并固定 TLS 探针计划
    Daemon->>Runtime: 建立身份、trace 与 membership
    Daemon->>Collect: 资源准入、采集绑定与探针挂载
    Daemon->>Runtime: 激活并持久化 trace
    Daemon-->>CTL: 注册成功
    opt 启用 seccomp
        CTL->>Daemon: 注册 seccomp listener
        Daemon-->>CTL: 注册成功
    end
    CTL->>Child: SIGCONT，写入并关闭环境管道
    Child->>Child: 等待管道 EOF，设置环境并 exec
    Child-->>CTL: 目标退出
    CTL->>Daemon: 移除 launch root
```

pidfd 是 launch 目标注册的身份依据。暂停及环境管道屏障使目标执行发生在准入和注册之后；准备或注册失败时 ctl 清理子进程与已建立状态。进程 fork/exec 的双坐标与身份时序见[进程身份运行时](components/process-identity-runtime.md)。相关决策见[pidfd 注册](../decisions/0002-use-pidfd-for-launch-registration.md)。

## 观测与交付

```mermaid
sequenceDiagram
    participant Source as Collector 或明文来源
    participant Identity as 身份与 Membership
    participant Runtime as 协议与语义运行时
    participant Recorder as Recording
    participant Storage as 主存储
    participant Consumer as 导出使用方
    Source->>Identity: 原始事件或明文来源身份
    Identity->>Runtime: 精确归属的事件或段
    Runtime->>Runtime: 协议恢复与语义投影
    Runtime->>Recorder: action、link、content、lineage、diagnostic
    opt 配置保留观测
        Recorder->>Storage: 尝试事务写入
        Storage-->>Recorder: 持久化结果
    end
    Recorder->>Consumer: 独立尝试发布
    Consumer-->>Recorder: 发布结果
    Recorder->>Recorder: 汇总 DeliveryReport
```

主存储和导出的结果分别记录；资源样本使用[资源组件](components/resource-profiling/design.md)的独立存储路径。消息解码或内容预算失败限制在相应消费者内，底层观测按自己的条件继续处理。

## 操作治理

下图表示需要用户态响应的控制通知。LSM 规则预先编译并发布到内核，操作在内核裁决，用户态异步接收审计；要求同步用户态决定的规则无法编译为该 LSM 路径。

```mermaid
sequenceDiagram
    participant Workload as 工作负载
    participant Boundary as 操作控制边界
    participant Control as 治理服务
    participant Identity as 身份与 Trace Runtime
    participant Policy as 规则或 control-decider
    Workload->>Boundary: 文件、命令或网络操作
    Boundary->>Control: 待裁决操作
    Control->>Identity: 解析稳定身份与归属
    Identity-->>Control: 操作上下文
    Control->>Policy: 在对应能力范围内裁决
    Policy-->>Control: 决策或局部错误
    Control-->>Boundary: 按已配置策略响应
    Boundary-->>Workload: 操作结果
```

不同控制后端各自实现拦截和响应机制。seccomp、fanotify、LSM 与网络控制的职责见[Core Runtime](components/core-runtime.md)；插件控制能力见[control-decider API](../reference/plugin-api/control-decider.md)。

## Hand-side 路由

```mermaid
sequenceDiagram
    participant Guest as actrail-sb
    participant Gateway as VSOCK gateway
    participant Ingest as GatewayIngestRuntime
    participant Route as Sandbox 路由
    participant Plugin as Sandbox 插件
    participant Evidence as Evidence DB
    Guest->>Gateway: VSOCK observation batch
    Gateway->>Ingest: TCP bounded frame
    Ingest->>Route: 校验后的 observation batch
    Route->>Plugin: 查询兴趣
    Plugin-->>Route: 匹配计划或错误
    alt 兴趣查询成功
        Route->>Plugin: 匹配的 observations
        Route->>Evidence: 未匹配的 observations
    else 兴趣查询失败
        Route->>Route: 局部错误处理
    end
```

同一批可以包含匹配项与未匹配项，互斥边界是单条 observation。插件产生的告警另行进入 Sandbox Alert DB。数据与身份定义见[隔离运行时](components/execution-isolation.md)。

## 查询与管理

```mermaid
sequenceDiagram
    actor Browser as 浏览器
    participant Web as actrailweb
    participant View as 查询视图
    participant Data as 本地主存储或 Cluster snapshot
    participant Daemon as actraild
    Browser->>Web: 查询 trace 或语义详情
    Web->>View: 校验分页、过滤与预算
    View->>Data: 只读查询
    Data-->>View: 记录与关系
    View-->>Web: 查询投影
    Web-->>Browser: API 响应
    opt 本地模式且加载 operator 配置
        Browser->>Web: 受支持的插件管理请求
        Web->>Daemon: Control socket 请求
        Daemon-->>Web: 操作结果
        Web-->>Browser: API 响应
    end
```

资源查询通过资源数据库接口读取样本；远端独占存储由接收端提供查询。Web 缓存、渐进时间线和正文预算见[Web Runtime](components/web-runtime.md)，前端状态与交付时序见[前端运行与交付](components/web-frontend/runtime-and-delivery.md)。
