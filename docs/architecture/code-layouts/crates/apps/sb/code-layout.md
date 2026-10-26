状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/sb/ # actrail-sb；Guest 内采集器、隔离观测运行时与本地控制的组合根
├── [x] Cargo.toml # Guest Linux 采集、VSOCK 发送、控制 socket 与运行时依赖
└── [x] src/ # CLI、静态配置、采集启动及有界关闭
    ├── [x] lib.rs # 暴露 CLI 与 daemon 启动能力
    ├── [x] bin/ # Guest 服务命令入口
    │   └── [x] actrail-sb.rs # 分派 init、daemon 与 connect 命令
    ├── [x] cli/ # 参数解析与向已有 daemon 发起连接请求
    │   ├── [x] mod.rs # 组织 CLI 模块并导出运行入口
    │   ├── [x] entry.rs # 装配配置、启动事件循环或执行客户端命令
    │   ├── [x] command.rs # 将参数转换成初始化、常驻运行或连接调用
    │   │   ├── [x] enum SbInvocation # 区分 init、daemon 和 connect 应用行为
    │   │   └── [x] struct SbConnectInvocation # 保存控制 socket、Host CID、端口和请求等待参数
    │   └── [x] client.rs # 通过 UDS 控制协议建立 Guest 到 Host 的观测连接
    │       └── [x] struct SandboxConnectClient # 发送 connect 请求并校验控制端回复
    └── [x] daemon/ # 采集资源、控制服务、信号与诊断生命周期
        ├── [x] mod.rs # 导出 daemon 配置、bootstrap 与生命周期入口
        ├── [x] bootstrap.rs # 构造采集来源、发送器和控制服务
        │   ├── [x] struct SandboxAgentDaemonBootstrap # start 验证配置并装配 Guest 运行时
        │   ├── [x] struct SandboxAgentDaemonProcess # 持有 agent、控制服务器与实例锁；shutdown 关闭采集和控制资源
        │   ├── [x] struct IoSource # 将 Guest Linux I/O 采集结果适配给观测运行时
        │   ├── [x] struct ResourceSource # 提供 Guest 资源快照采样
        │   ├── [x] struct PressureSource # 提供内存压力观测
        │   └── [x] struct WorkloadSource # 提供 Guest workload 资源观测
        ├── [x] config.rs # 拥有采集、采样、队列、发送、控制和诊断子配置
        │   ├── [x] struct SbDaemonConfig # load 解析静态配置；validate 构造可启动的配置对象
        │   ├── [x] struct ValidatedSbDaemonConfig # 保存校验后的运行时、采集和控制配置
        │   └── [x] struct SbDaemonConfigOverrides # 为 init 明确覆盖根进程匹配、socket 和实例锁路径
        ├── [x] instance_lock.rs # 通过本地文件锁约束单实例 daemon
        │   └── [x] struct InstanceLock # acquire 取得锁并用对象生命周期持有锁文件
        ├── [x] lifecycle.rs # 合并关闭信号与控制服务健康事件
        │   ├── [x] struct DaemonEventOwner # 持有事件等待资源，wait 唤醒诊断或关闭流程
        │   └── [x] enum DaemonEvent # 表达 daemon 主循环需要处理的生命周期事件
        └── [x] output.rs # 输出就绪状态、连接结果、采集失败和周期诊断
            ├── [x] struct SbOutput # 持有周期诊断状态并限频输出运行信息
            └── [x] struct CollectorDiagnostics # 汇总采集失败数量与内核诊断快照
```
