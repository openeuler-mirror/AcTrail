状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/tls_payload_sync/ # 定义注入运行库与宿主之间的探针计划、环境配置和有界事件传输。
└── [x] src/ # 启动环境、传输协议及运行库事件客户端。
    ├── [x] lib.rs # 导出注入配置、计划描述和同步通信接口。
    ├── [x] env.rs # 生成运行库所需的环境变量、过滤规则和流控参数。
    │   ├── [x] struct RuntimeEnvConfig # 携带通信位置、过滤与脱敏设置。
    │   ├── [x] struct RuntimeFlowControlConfig # 设置协议嗅探、头部、大流量及未知流预算和最大流数量。
    │   └── [x] struct EventFilter # 选择 target、payload 和 decision 事件是否上报。
    ├── [x] plan.rs # 编解码可传递给注入运行库的探针计划。
    │   └── [x] struct RuntimePlanDescriptor # 保存运行库执行探针所需的目标与探针点描述。
    ├── [x] lookup.rs # 经同步连接向宿主查询运行库计划。
    │   ├── [x] struct PlanLookupRequest # 提交待匹配的目标文件信息。
    │   └── [x] enum PlanLookupResponse # 返回探针计划或未命中结果。
    ├── [x] launch.rs # 选择注入库、校验 native 计划并构造目标启动环境。
    │   ├── [x] enum RuntimeLibraryPath # 区分自动发现与指定路径的注入运行库。
    │   └── [x] struct RuntimeLibrarySet # 保存 glibc 和可选 musl 注入库路径。
    ├── [x] runtime.rs # 拥有同步连接、字节预算队列和后台写入线程。
    │   ├── [x] struct EventClient # 将调用线程中的事件交给有界队列，再由写入线程发送。
    │   │   ├── [x] send # 提交 SyncEvent，并返回队列接收结果。
    │   │   └── [x] close # 停止接收并按关闭期限等待已排队事件发送。
    │   ├── [x] struct EventQueue # 控制排队字节数、生产者唤醒和队列关闭。
    │   └── [x] struct EventWriter # 组合事件帧并执行批量 socket 写入。
    ├── [x] protocol/ # 注入端和宿主共用的二进制帧格式。
    │   ├── [x] header.rs # 解析帧头并区分消息种类。
    │   │   └── [x] struct FrameHeader # 保存帧类型与正文长度，编解码时校验固定协议版本。
    │   └── [x] codec.rs # 在类型化消息与帧字节之间转换。
    │       ├── [x] enum SyncMessage # 表示同步事件、计划查询及查询响应。
    │       └── [x] struct FrameCodec # 编解码同步协议帧并校验输入边界。
    └── [x] target.rs # 识别目标 ELF 使用的运行库族及文件代际。
        ├── [x] enum LibcFamily # 区分 glibc 与 musl 运行库族。
        └── [x] struct TargetRuntime # 保存目标路径、可选 ELF 解释器和运行库族；无解释器表示静态目标。
```
