状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/plugins/ # 告警转发与隔离资源告警的内置插件实现
├── [x] alert_forwarding/ # 依据类别过滤告警，向当前连接提供非阻塞投递入口
│   └── [x] src/ # 转发配置、连接代际与投递状态
│       ├── [x] lib.rs # 导出配置、插件运行时和投递结果
│       ├── [x] runtime.rs # 管理当前转发连接、启停状态和丢弃计数
│       │   ├── [x] struct AlertForwardingPlugin # activate 安装当前连接；try_publish 投递告警；disable_if_generation 只停用匹配代际
│       │   ├── [x] struct ConnectionGeneration # 用非零代际区分转发连接
│       │   ├── [x] struct ForwardingItem # 携带待交付告警与连接代际
│       │   ├── [x] struct AlertForwardingPluginStatus # 汇总配置、连接及投递状态
│       │   └── [x] enum AlertForwardingSubmitOutcome # 表达投递接收、过滤或丢弃结果
│       ├── [x] config.rs # 解析 JSON 转发配置并检查类别数量、格式和重复项
│       │   └── [x] struct AlertForwardingConfig # 持有启用开关、全部类别开关和指定类别列表
│       ├── [x] config_owner.rs # 将插件配置的读取和更新交给配置所有者
│       │   ├── [x] trait AlertForwardingConfigOwner # 提供持久配置读取与更新接口
│       │   └── [x] struct AlertForwardingConfigFileOwner # 从配置文件读取并保存插件配置
│       └── [x] filter.rs # 编译并匹配允许转发的告警类别
│           └── [x] struct CategoryFilter # 保存类别过滤条件
└── [x] sandbox_resource_alert/ # 消费 Guest 资源观测并产生独立 Sandbox 告警
    └── [x] src/ # 阈值配置、插件注册与来源状态
        ├── [x] lib.rs # 导出插件和配置，并重导出告警写入契约
        ├── [x] plugin.rs # 注册观测兴趣、处理资源样本并提交告警
        │   └── [x] struct SandboxResourceAlertPlugin # register 接入插件路由；publish_config 更新阈值配置
        ├── [x] config.rs # 解析资源告警阈值与来源状态容量
        │   └── [x] struct SandboxResourceAlertConfig # 保存 Guest 资源阈值和状态预算
        └── [x] state.rs # 按观测来源保存计算告警所需的资源历史
            ├── [x] struct SourceStateTable # 管理来源状态及有界容量
            └── [x] struct SourceState # 保存单个来源的资源变化和告警计算状态
```
