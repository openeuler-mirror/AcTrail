状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/alert_proxy/ # actraild-alert-proxy；接收本地规范化告警并向通过认证的订阅连接广播
├── [x] Cargo.toml # 告警代理协议、网络服务和配置依赖
└── [x] src/ # 本地摄入、订阅认证、匹配分发与连接生命周期
    ├── [x] lib.rs # 暴露代理 bootstrap 和配置
    ├── [x] bin/ # 独立代理进程入口
    │   └── [x] actraild-alert-proxy.rs # 读取配置、启动代理并处理进程关闭
    ├── [x] startup/ # 监听资源构造与成组关闭
    │   ├── [x] mod.rs # 导出配置与启动入口
    │   ├── [x] config.rs # 校验 UDS 摄入、TCP 订阅、认证和容量参数
    │   │   ├── [x] struct AlertProxyConfig # 组合 daemon 摄入与订阅配置
    │   │   ├── [x] struct DaemonIngressConfig # 限制本地连接与接收帧预算
    │   │   └── [x] struct SubscriberConfig # 保存订阅认证、连接和心跳约束
    │   └── [x] bootstrap.rs # 按配置装配注册表、广播器与两个监听服务
    │       ├── [x] struct AlertProxyBootstrap # start 验证并创建代理运行时
    │       └── [x] struct AlertProxyRuntime # 持有摄入、订阅与广播资源；shutdown 回收各工作线程
    ├── [x] daemon_ingress.rs # 校验 Unix peer 并接收有界告警 frame
    │   ├── [x] struct DaemonIngressServer # 管理 daemon 监听与接收工作资源
    │   └── [x] struct DaemonConnection # 持有单条连接的输入与解析状态
    ├── [x] broadcaster.rs # 将已接收告警提交给独立广播工作单元
    │   └── [x] struct AlertBroadcaster # 以有界队列连接摄入与订阅注册表，局部处理交付失败
    ├── [x] registry.rs # 维护有效订阅及每个订阅方的交付状态
    │   ├── [x] struct SubscriberRegistry # 按订阅条件匹配告警并管理连接登记与移除
    │   ├── [x] struct SubscriberHandle # 表示单个订阅连接的分发句柄
    │   └── [x] struct Subscription # 保存告警筛选条件
    ├── [x] subscriber/ # TCP 会话认证、请求处理和心跳
    │   ├── [x] mod.rs # 组织订阅监听与内部会话模块
    │   ├── [x] listener.rs # 接受订阅连接并启动会话
    │   │   └── [x] struct SubscriberServer # start 创建监听工作单元；shutdown 关闭监听及会话资源
    │   ├── [x] session.rs # 读取协议请求并发送响应、告警和心跳
    │   │   ├── [x] struct SubscriberSession # run 执行认证、订阅更新及断开后的注销
    │   │   └── [x] struct SessionWriter # 协调同一连接上的消息写出
    │   ├── [x] auth.rs # 校验客户端提交的访问令牌
    │   │   └── [x] struct TokenVerifier # 保存允许的 token 并通过 accepts 返回认证结果
    │   └── [x] activity.rs # 记录最近活动、待确认 ping 和超时动作
    │       ├── [x] struct SessionActivity # record_request/accept_pong 更新连接活动；heartbeat_action 决定后续行为
    │       └── [x] enum HeartbeatAction # 表达等待、发送心跳或终止会话的选择
    └── [x] diagnostics.rs # 统一输出代理连接、队列和协议错误诊断
        └── [x] struct ProxyDiagnostics # 汇总诊断输出入口
```
