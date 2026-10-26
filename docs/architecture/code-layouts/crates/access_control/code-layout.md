状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/access_control/ # 文件、命令和网络策略管理，以及共享进程范围的 LSM 执行
├── [x] build.rs # 编译访问控制 BPF 对象
│   └── [x] struct LsmObjectBuilder # 生成不同进程范围存储方式的 LSM 对象
├── [x] bpf/ # 内核操作拦截、策略匹配和审计输出
│   ├── [x] access_control.bpf.c # 挂载进程、执行、文件和网络 LSM 程序
│   ├── [x] abi.h # 定义策略、范围和审计的内核 ABI
│   ├── [x] scope.h # 读取共享进程范围与待执行绑定
│   ├── [x] policy.h # 在内核中查询策略并生成裁决
│   └── [x] directory.h # 提供目录操作的路径判断
└── [x] src/ # 用户态规则管理、策略发布与审计消费
    ├── [x] lib.rs # 导出 policy 与 lsm 能力
    ├── [x] lsm/ # 持有 LSM 对象、策略映射和审计环形缓冲
    │   ├── [x] runtime.rs # 复用观测运行时的进程范围 map 并挂载 LSM 程序
    │   │   └── [x] struct LsmRuntime # attach 创建对象和 links；poll_fd、drain 提供有界审计消费
    │   ├── [x] controller.rs # 管理策略存储并协调内核策略更新
    │   │   ├── [x] struct LsmController # publish_file_rules 发布文件规则；remove_owner 清理插件所属规则
    │   │   └── [x] struct PolicyStores # 聚合命令和网络策略存储
    │   ├── [x] compiler.rs # 将用户态规则编译为内核策略快照
    │   │   └── [x] struct PolicySnapshot # 保存一次发布的完整策略内容
    │   ├── [x] publisher.rs # 更新策略 map 中的快照槽位
    │   │   └── [x] struct PolicyPublisher # 持有策略 map 句柄与容量
    │   ├── [x] audit.rs # 将内核审计字节解码为操作记录
    │   │   └── [x] struct LsmAuditEvent # 表达操作、进程和决策的审计结果
    │   ├── [x] hosts.rs # 接入插件策略查询与修改能力
    │   └── [x] hosts/ # 各控制域的插件宿主适配
    │       ├── [x] command.rs # 向插件提供命令规则读取、校验和更新
    │       └── [x] network.rs # 向插件提供网络规则读取、校验和更新
    └── [x] policy/ # 持久规则、动态规则及规则 owner 生命周期
        ├── [x] path.rs # 规范化策略中的路径范围
        │   └── [x] struct PolicyPath # 表示用于匹配和覆盖判断的路径
        ├── [x] file/ # 文件访问规则、目录范围与优先级
        │   ├── [x] store.rs # 加载文件规则并处理动态变更
        │   │   └── [x] struct EnforcementRules # load、validate_apply、apply 和 remove_owner 管理有效规则
        │   ├── [x] types.rs # 文件规则与裁决结果
        │   │   ├── [x] struct EnforcementRule # 表示文件操作的匹配条件与决策
        │   │   └── [x] enum RuleDecision # 表达文件规则命中的决策
        │   ├── [x] model.rs # 保存规则层级、身份及 owner 信息
        │   │   └── [x] struct StoredPolicyRule # 聚合已安装规则及其管理身份
        │   └── [x] validation.rs # 检查新增规则的范围与授权
        │       └── [x] struct FilePolicyValidation # 验证文件策略更新
        ├── [x] command/ # 可执行文件与参数范围的命令策略
        │   ├── [x] store.rs # 加载规则并按可执行文件、参数匹配
        │   │   └── [x] struct CommandPolicyStore # find、match_dry_run、apply、remove_owner 管理命令规则
        │   ├── [x] rule.rs # 保存命令规则与安装者信息
        │   │   └── [x] struct StoredCommandRule # 表达可执行文件、参数模式和规则 owner
        │   └── [x] validation.rs # 检查命令规则的授权范围
        │       └── [x] struct CommandRuleDraftValidator # 验证待安装的命令规则
        └── [x] network/ # 远端地址与端口范围的网络策略
            ├── [x] store.rs # 加载规则并按网络端点匹配
            │   └── [x] struct NetworkPolicyStore # find、match_dry_run、apply、remove_owner 管理网络规则
            ├── [x] rule.rs # 保存网络规则与安装者信息
            │   └── [x] struct StoredNetworkRule # 表达地址、端口、决策和规则 owner
            └── [x] validation.rs # 检查网络规则的范围和授权
                └── [x] struct NetworkRuleValidator # 验证待安装的网络规则
```
