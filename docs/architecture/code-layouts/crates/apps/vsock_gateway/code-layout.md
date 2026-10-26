状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/vsock_gateway/ # actrail-vsock-gateway；为不同 VMM 装配 Guest 监听端与 daemon 上游连接
├── [x] Cargo.toml # 网关运行时、VSOCK adapter 与配置解析依赖
└── [x] src/ # 可执行入口和网关后端选择
    ├── [x] lib.rs # 导出配置与 bootstrap
    ├── [x] bin/ # 启动与初始化命令入口
    │   └── [x] actrail-vsock-gateway.rs # 解析后端参数，生成配置或启动网关运行时
    │       └── [x] enum Command # 选择 init 与网关常驻运行
    └── [x] startup/ # 配置校验、监听器解析与运行时组装
        ├── [x] mod.rs # 导出启动配置、后端选择和 bootstrap
        ├── [x] bootstrap.rs # 将应用配置转换为网关核心运行时
        │   └── [x] struct GatewayBootstrap # start 解析配置并启动 GatewayRuntime
        ├── [x] config.rs # 拥有监听、上游连接与队列/会话参数
        │   ├── [x] struct GatewayAppConfig # load 读取配置；into_runtime 产生经校验的 GatewayConfig
        │   └── [x] struct GatewayConfigOverrides # 初始化时显式覆盖 VMM backend、socket、CID 与 daemon 地址
        └── [x] backend/ # 各 VMM 监听端点到统一 VsockListenerConfig 的适配
            ├── [x] mod.rs # 根据 backend 选择具体端点并校验允许的参数
            │   ├── [x] enum GatewayBackend # 区分 Native、Firecracker 与 Cloud Hypervisor
            │   └── [x] enum ListenerSection # 保存后端专属配置；resolve 返回统一监听配置
            ├── [x] native.rs # 原生 AF_VSOCK CID 与 port 监听
            │   └── [x] struct NativeEndpoint # 持有 CID/端口并构造监听器参数
            ├── [x] firecracker.rs # 根据 Firecracker UDS 路径和端口解析监听端点
            │   └── [x] struct FirecrackerEndpoint # resolve 验证端点路径；listener 产生转发监听配置
            └── [x] cloud_hypervisor.rs # 使用 Cloud Hypervisor 的 Unix socket 端点
                └── [x] struct CloudHypervisorEndpoint # 校验路径并构造对应监听配置
```
