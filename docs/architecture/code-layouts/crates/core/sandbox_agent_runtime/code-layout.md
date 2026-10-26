状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/sandbox_agent_runtime/ # 在 Guest 内协调观测采样与工作单元会话，将批次通过可替换传输端口发送。
└── [x] src/ # 配置、数据源端口、线程生命周期及会话隔离。
    ├── [x] lib.rs # 导出 daemon、控制句柄、数据源和传输接口。
    ├── [x] config.rs # 校验采样周期、队列、批次、重连和线程预算。
    │   └── [x] struct SandboxAgentConfig # 启动采样及会话线程所需的有效配置。
    ├── [x] ports.rs # 将进程 IO、Guest 资源、压力与工作负载采样隔离在来源端口后。
    │   ├── [x] trait ProcessIoSource # 建立基线并读取进程 IO 观测。
    │   ├── [x] trait GuestResourceSource # 采样 Guest 资源观测。
    │   ├── [x] trait WorkloadCgroupSource # 采样工作负载 cgroup 指标。
    │   └── [x] trait SandboxTransportFactory # 按会话需要建立可读写的网关连接。
    ├── [x] daemon/ # 组合采样与会话线程，接收工作单元控制请求。
    │   ├── [x] owner.rs # 建立基线、共享状态和有界队列并拥有线程集合。
    │   │   └── [x] struct SandboxAgentDaemon # start 时验证来源；拥有停止信号、投递门和控制句柄。
    │   ├── [x] control.rs # 在控制调用方和会话拥有者之间传递有限命令。
    │   │   └── [x] struct SandboxAgentControlHandle # 提供工作单元控制入口，命令经队列交给会话线程。
    │   └── [x] workers.rs # 驱动各来源采样线程及基线重置。
    │       └── [x] struct WorkerSet # 管理采样线程句柄及停止回收。
    ├── [x] session/ # 拥有连接、重连目标、工作单元状态和批次发送次序。
    │   ├── [x] owner.rs # 串行处理会话控制与连接生命周期。
    │   │   └── [x] struct SessionOwner # 持有活动连接和重连目标，使会话变更有唯一执行者。
    │   ├── [x] protocol.rs # 负责会话握手及协议发送。
    │   │   └── [x] struct SessionProtocol # 将连接读写转换为 Sandbox 协议交互。
    │   ├── [x] status.rs # 发布当前会话状态，供控制请求和诊断读取。
    │   └── [x] wake.rs # 唤醒等待中的会话线程处理命令或新增观测。
    ├── [x] delivery/ # 以连接代际隔离排队观测，防止跨会话交付。
    │   └── [x] mod.rs # 定义投递门、信封、有界队列及准入结果。
    │       ├── [x] struct ConnectionGate # 发布当前可交付的连接代际。
    │       ├── [x] struct DeliveryEnvelope # 将观测绑定到采集时连接代际。
    │       └── [x] struct DeliveryPipeline # 给采样线程共享投递入口，按门状态和容量接纳观测。
    └── [x] status.rs # 采样、队列、会话和丢弃计数的诊断快照。
        └── [x] struct SandboxAgentSnapshot # 汇总运行状态，供应用控制界面读取。
```
