状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/ctl/ # actrailctl；生成配置、查询 daemon、注册受控进程并监督工作负载
├── [x] Cargo.toml # 控制客户端、配置与 Linux 启动能力的应用依赖
└── [x] src/ # 命令解析、控制传输与本地进程生命周期
    ├── [x] lib.rs # 暴露命令分派、环境入口和回复格式化
    ├── [x] bin/ # 控制工具可执行入口
    │   └── [x] actrailctl.rs # 调用应用入口并向调用者返回退出码
    ├── [x] args.rs # 将 CLI 参数转换成带配置路径的命令
    │   ├── [x] struct CtlInvocation # 保存 operator 配置位置、控制 socket 与所选命令
    │   ├── [x] enum CtlCommand # 区分 launch、track、doctor、配置生成与清理操作
    │   └── [x] enum InitMode # 选择生成配置的范围和预设
    ├── [x] entry.rs # 加载配置、创建客户端并执行一次命令
    ├── [x] entry/ # 默认配置与初始化文件写入
    │   ├── [x] init.rs # 合并初始化选项并生成 operator/profile 配置
    │   ├── [x] profile.toml # 初始化时的采集 profile 模板
    │   └── [x] complete.toml # 初始化时的完整配置模板
    ├── [x] dispatch.rs # 将 CtlCommand 路由到控制请求或本地 launch 流程
    ├── [x] transport.rs # 为命令分派隔离具体 UDS 传输
    │   └── [x] trait ControlClientPort # send 传递控制命令；send_launch_track_add 同时借用 pidfd 完成启动注册
    ├── [x] launch.rs # 探测权限与 TLS 计划，注册停止中的子进程后继续执行
    │   └── [x] struct LaunchRequest # 聚合工作负载 argv、profile、权限与采集注入配置
    ├── [x] launch/ # 受控 fork/exec、监听器交接、环境注入与退出监督
    │   ├── [x] controlled.rs # 持有受控子进程和继续执行通道
    │   │   ├── [x] enum ChildSetup # 区分普通启动与 seccomp 初始化路径
    │   │   └── [x] struct ControlledChild # spawn 持有 pidfd；continue_with_envs 放行；wait_with_monitor 监督；terminate 清理失败启动
    │   ├── [x] controlled/ # 内核进程创建原语与 pidfd 获取
    │   │   └── [x] spawn.rs # clone3 及兼容创建路径
    │   │       └── [x] enum PidfdSpawn # 区分父进程持有的 pidfd 与子进程执行分支
    │   ├── [x] seccomp.rs # 安装通知过滤器并把 listener 交给 daemon
    │   │   └── [x] struct SeccompSetup # 保存过滤规则与预留 listener FD；install 在子进程中建立通知边界
    │   ├── [x] seccomp/ # 按采集与治理能力生成 syscall 规则
    │   │   └── [x] rules.rs # 合并所需通知 syscall 并生成 BPF 过滤程序
    │   ├── [x] injection.rs # 根据 TLS backend 选择注入实现并约束准备、环境生成接口
    │   │   ├── [x] trait LaunchInjection # 将准备资源与生成子进程环境封装为同一注入行为
    │   │   ├── [x] struct InjectionResources # 持有启动准备阶段取得的注入资源
    │   │   └── [x] struct InjectionFactory # create 按 PayloadTlsConfig 装配具体注入器
    │   ├── [x] injection/ # TLS-sync 与空注入器的具体实现
    │   │   ├── [x] tls_sync.rs # 准备同步采集动态库与目标进程环境
    │   │   └── [x] noop.rs # 无需注入的采集 backend 使用空准备路径
    │   ├── [x] target.rs # 解析待执行目标及 agent 接入配置
    │   │   └── [x] struct LaunchTargetResolver # resolve 输出实际启动目标与参数
    │   ├── [x] permission_policy.rs # 将本地权限设置与 daemon 裁决转换成启动行为
    │   ├── [x] java_agent.rs # 为 Java 工作负载准备 agent 启动选项
    │   ├── [x] suppress.rs # 管理需要从工作负载观测中抑制的可继承描述符
    │   └── [x] timing.rs # 记录启动各阶段耗时用于诊断
    ├── [x] platform_probe.rs # 探测 seccomp、TLS-sync 动态库与 socket 可用性
    │   └── [x] struct LaunchPlatformReport # 汇总权限裁决所需平台能力与不可用原因
    ├── [x] process_ref.rs # 为挂接命令构造带坐标信息的进程引用
    ├── [x] tls_plan.rs # 查询 daemon 对目标程序选择的 TLS 探针计划
    │   └── [x] struct QueriedLaunchTlsPlan # 保留计划查询结果供输出与 launch 使用
    ├── [x] clean.rs # 根据配置定位并清理 AcTrail 生成的本地文件
    │   └── [x] struct CleanArtifacts # 汇总待清理目标并控制清理过程
    └── [x] output.rs # 把控制回复转换为终端可读表示
```
