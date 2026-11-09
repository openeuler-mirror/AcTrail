状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/adapters/collectors/ebpf/ # 按固定配置一次准备观测与治理 runtime，保留到 daemon 退出
├── [x] build.rs # 编译观测组合对象，选择部署内核适用的编译变体
├── [x] build/
│   └── [x] environment.rs # 选择事件通道、launch 绑定和原子操作的内核变体
├── [x] bpf/ # 按程序职责组织源码，由 live_observation.bpf.c 组合编译
└── [x] src/
    ├── [x] lib.rs # 构造采集器并保存绑定状态与唯一 runtime 准备状态
    │   └── [x] struct EbpfCollector # 实现 CollectorInstance，管理目标绑定、观测及明文采集
    ├── [x] capability_probe.rs # 探测环境，实际能力由准备成功的 runtime 决定
    ├── [x] maps.rs # 保存 trace 绑定和跟踪进程的用户态状态
    ├── [x] procfs.rs # 查询进程身份和进程树
    ├── [x] collector/
    │   ├── [x] runtime.rs # 查询与使用已准备 runtime，返回采集数据和诊断
    │   ├── [x] runtime/
    │   │   └── [x] preparation.rs # 一次准备、保存结果并核验后续能力要求
    │   │       ├── [x] enum RuntimePreparation # 未准备、无需求、失败或持有唯一成功 runtime
    │   │       ├── [x] prepare_capabilities # 准备配置需求并保存结果，重复调用返回保存结果
    │   │       ├── [x] check_preflight_capabilities # 从实际 runtime 检查 profile 的 Required 能力
    │   │       └── [x] ensure_runtime_for_requests # 绑定前核验能力，保留共享程序及 maps
    │   ├── [x] instance.rs # 接入、解绑和轮询，trace 结束只清理目标状态
    │   ├── [x] access_control/bridge.rs # 设置启动治理配置并只读检查和使用 LSM runtime
    │   ├── [x] dynamic_tls.rs # 为具体目标安装 TLS 探针
    │   ├── [x] dynamic_go_tls.rs # 为具体目标安装 Go TLS 探针
    │   └── [x] net_aggregation.rs # 聚合网络观测并按阈值刷新
    ├── [x] loader.rs # 定义加载配置与 runtime 资源所有权
    │   ├── [x] struct EbpfProgramLoader # 按配置加载观测组合对象并装配 LSM
    │   └── [x] struct EbpfRuntime # 持有消费者、Object、Links、绑定 maps、TLS 状态和可选 LSM runtime
    └── [x] loader/
        ├── [x] attach_plan/ # 将配置需求转换为固定程序集合及 payload 开关
        │   ├── [x] implementation.rs # AttachPlan 选择程序并从实际挂载结果推导能力
        │   └── [x] programs.rs # 声明各能力所需的程序集合
        ├── [x] program/ # 对象、环境、tracepoint 和加载诊断
        ├── [x] runtime/ # 初始化资源以及目标绑定、采集和诊断操作
        │   ├── [x] initialization.rs # 创建共享 maps、消费者、挂载和 LSM，记录实际加载尝试
        │   ├── [x] implementation.rs # 使用长期 runtime 执行目标及事件操作
        │   ├── [x] process_identity.rs # 查询与解析进程身份
        │   ├── [x] observation_depth.rs # 管理目标观测范围
        │   └── [x] loss.rs # 汇总采集损失
        ├── [x] consumer.rs # 持续消费共享事件通道
        ├── [x] ring_decode.rs # 解码内核事件公共头并分发领域载荷
        ├── [x] launch_binding.rs # 管理暂停 launch 目标的待执行绑定
        ├── [x] file_io/ # 读取和清理文件 I/O 汇总
        └── [x] tls/ # 使用已有程序和 maps，管理目标探针与待完成采集
```
