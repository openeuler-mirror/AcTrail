# AcTrail 当前代码布局

本文描述当前仓库的物理代码结构。执行隔离组件的后续稳定布局见 [执行隔离目标代码布局](execution-isolation/target-layout.md)，结构约束见 [执行隔离代码布局设计约束](execution-isolation/design-constraints.md)。

## 1. Workspace

```text
AcTrail/
├── Cargo.toml                              # Rust workspace package 清单与统一版本
├── AGENTS.md                               # 仓库软件工程规则
├── crates/
│   ├── apps/                               # C4 Container：进程入口与 composition roots
│   │   ├── daemon/                         # actraild
│   │   ├── sb/                             # actrail-sb daemon、同 binary CLI 与进程生命周期
│   │   ├── vsock_gateway/                  # actrail-vsock-gateway
│   │   ├── alert_proxy/                    # actraild-alert-proxy
│   │   ├── ctl/                            # actrailctl
│   │   ├── cluster/                        # actrailcluster
│   │   ├── view/                           # actrailviewer
│   │   └── web/                            # actrailweb 与 Vue frontend
│   ├── contracts/                          # 跨 C4 Component 的 DTO、ports 与 wire contracts
│   ├── core/                               # C4 Component 的领域状态、owners 与生命周期
│   ├── adapters/                           # eBPF、procfs、UDS、VSOCK、TCP、SQLite 等技术实现
│   ├── plugins/                            # builtin plugin implementations
│   ├── storage/                            # 主 Storage facade/factory/SQLite adapter
│   ├── export/                             # Export Runtime 与 OTEL adapters
│   ├── recording/                          # Recording Writer
│   ├── plugin/abi/                         # Plugin Host 稳定 ABI
│   └── tools/                              # 诊断与探测工具
├── tests/
│   ├── v2/                                 # 唯一允许使用的 release-binary 测试体系
│   └── ...                                 # 其余目录均过时，禁止复用
├── deploy/                                 # 安装、配置与运行资产
├── docs/                                   # C4、代码布局与设计文档
├── examples/                               # 插件与 trace 示例
└── scripts/                                # 构建、安装、打包与 benchmark
```

依赖方向以 app 作为最外层组装根：app 依赖 contracts、core 和 adapters；core 依赖 contracts；adapter 实现 contract 或 runtime port。`tests/v2/` 是唯一允许新增、维护和执行的测试根。

## 2. 当前执行隔离 Containers

```text
Guest workload
  → actrail-sb
  → AF_VSOCK
  → actrail-vsock-gateway
  → TCP
  → actraild / GatewayIngestRuntime
  → sandbox plugin 或独立 Sandbox Evidence DB
```

`actrail-sb`、`actrail-vsock-gateway` 和 `actraild` 是相互独立的 app crates。Guest observation 不进入既有 Ingest、Identity、Trace、Semantic、Recording、Export 或主 Storage 链路。

## 3. 当前 actrail-sb Container

```text
crates/apps/sb/
├── Cargo.toml                       # C4 Container actrail-sb 的组装依赖
└── src/
    ├── lib.rs                       # Container facade；导出 CLI 入口、daemon bootstrap 与静态配置
    ├── bin/actrail-sb.rs            # 同一 binary 入口；以 CLI 返回码结束进程
    ├── cli/
    │   ├── mod.rs                   # C4 Sandbox Control CLI facade
    │   ├── entry.rs                 # daemon/connect/init 分派；daemon 事件循环
    │   ├── command.rs               # CLI 参数与运行时 endpoint 值映射
    │   └── client.rs                # C4 Guest-local Control Client 组装与结果解释
    └── daemon/
        ├── mod.rs                   # C4 Sandbox Agent Daemon composition facade
        ├── config.rs                # 静态 TOML、默认 profile、校验与 init 输出
        ├── bootstrap.rs             # collector、sampler、runtime、VSOCK factory 与 UDS server 组装
        ├── instance_lock.rs         # Guest kernel 范围单 daemon flock owner
        ├── lifecycle.rs             # signalfd + ppoll 的 signal/control-health/diagnostics owner
        └── output.rs                # ready、CLI 结果与集中周期诊断输出
```

`actrail-sb daemon` 与 `actrail-sb connect` 属于同一 binary。CLI 只连接 Guest-local UDS；daemon 独占 Guest-only eBPF、资源 reader、采集 workers、VSOCK session 和实例锁。

## 4. 当前 actrail-sb Components

### 4.1 Sandbox Agent Runtime

```text
crates/core/sandbox_agent_runtime/src/
├── lib.rs                           # C4 Sandbox Agent Runtime facade
├── config.rs                        # poll、queue、batch、silence、reconnect、control deadline 与 worker 配置
├── ports.rs                         # ProcessIoSource、GuestResourceSource、transport/connection ports
├── status.rs                        # 可选低开销 runtime counters snapshot
├── daemon/
│   ├── mod.rs                       # C4 Daemon Owner facade
│   ├── owner.rs                     # 预热 sources、预分配队列/batch、启动 workers、统一关闭
│   ├── control.rs                   # SandboxControlPort、Busy、daemon control timeout 与状态入口
│   └── workers.rs                   # I/O/resource workers、baseline command 与 join owner
├── delivery/
│   └── mod.rs                       # C4 Connection Gate：原子 generation、有界 try_send 与旧队列丢弃
└── session/
    ├── mod.rs                       # C4 Sandbox Link Session facade
    ├── owner.rs                     # endpoint、连接、握手、batch、Heartbeat、重连与 gate 切换 owner
    ├── protocol.rs                  # SB link handshake、batch 与 Heartbeat 写入
    ├── status.rs                    # endpoint/session/publication 的一致状态 snapshot
    └── wake.rs                      # command/delivery 合并唤醒令牌
```

### 4.2 Guest Linux Collector

```text
crates/adapters/collectors/sandbox_linux/
├── Cargo.toml                       # Guest-only collector package与依赖
├── build.rs                         # 构建 Guest-only BPF object
├── bpf/
│   ├── sandbox_io.bpf.c             # C4 Process I/O Collector 的 kernel programs/maps
│   └── sandbox_bpf_helpers.h        # BPF 辅助定义
└── src/
    ├── lib.rs                       # collector 与 resource reader facade
    ├── config.rs                    # root names、procfs、map capacities 与 refresh 配置
    ├── collector.rs                 # process I/O poll cycle 与独立 resource owner
    ├── ebpf.rs                      # root lineage、aggregate baseline 与 kernel diagnostics
    ├── procfs.rs                    # boot id、comm、start time 与 lineage discovery
    ├── resource.rs                  # C4 Guest Resource Sampler：CPU/memory/oom_kill snapshot
    └── error.rs                     # 启动与运行采集错误
```

### 4.3 Shared Collector Attachment Adapter

该adapter不是独立C4 Component。

它为Brain侧eBPF Collector和Guest侧Process I/O Collector提供相同的标准tracepoint挂载机制。

```text
crates/adapters/collectors/libbpf_tracepoint/
├── Cargo.toml                       # 共享的轻量libbpf attach adapter
└── src/
    ├── lib.rs                       # 最小 facade
    ├── attacher.rs                  # 标准 tracepoint 分类、tracefs、perf event 与 ioctl attach
    └── error.rs                     # attach阶段错误
```

### 4.4 Guest-local Control

```text
crates/contracts/sandbox_control/src/
├── lib.rs                           # C4 Control Client/Server contract facade
├── endpoint.rs                      # host CID 与 VSOCK port 值对象
├── command.rs                       # Connect command
├── response.rs                      # success/rejection 与 Busy 等结果
├── status.rs                        # daemon/session/publication 状态
└── port.rs                          # SandboxControlPort

crates/adapters/sandbox_control/uds/src/
├── lib.rs                           # C4 Guest-local Control adapter facade
├── client.rs                        # CLI 单 request/response UDS client
├── server.rs                        # listener、health fd 与 server handle
├── runtime.rs                       # 非阻塞 poll owner、连接集合与 admission
├── dispatcher.rs                    # 单槽有界、非阻塞admission的单worker dispatcher
├── connection.rs                    # 单命令 connection phase、deadline 与有界 buffer
├── codec.rs                         # 有界本地二进制 frame codec
└── error.rs                         # bind/connect/read/write/dispatch 错误
```

### 4.5 VSOCK Data Link

```text
crates/contracts/sandbox_link/vsock/src/
├── lib.rs                           # C4 Sandbox Link contract facade
├── frame.rs                         # SbHello/SbWelcome/Heartbeat/ObservationBatch
├── stream.rs                        # frame 边界恢复
├── batch_codec.rs                   # observation batch 紧凑 codec
└── error.rs                         # protocol errors

crates/adapters/sandbox_link/vsock/src/
├── lib.rs                           # C4 VSOCK adapter facade
├── client.rs                        # actrail-sb AF_VSOCK client factory
├── connection.rs                    # Read/Write connection owner
├── listener.rs                      # gateway backend-neutral listener
├── kernel_vsock.rs                  # native AF_VSOCK syscalls
└── unix_stream.rs                   # Firecracker/Cloud Hypervisor Unix endpoints
```

## 5. 当前 Host 执行隔离结构

```text
crates/apps/vsock_gateway/src/
├── lib.rs                           # C4 actrail-vsock-gateway facade
├── bin/actrail-vsock-gateway.rs     # gateway binary 入口
└── startup/
    ├── mod.rs                       # startup facade
    ├── config.rs                    # listener、session、upstream 与容量配置
    ├── bootstrap.rs                 # runtime 与 listener 组装
    └── backend/
        ├── mod.rs                   # backend 选择
        ├── firecracker.rs           # 主线 uds_path + port endpoint
        ├── native.rs                # 可选 AF_VSOCK
        └── cloud_hypervisor.rs      # 可选完整 Unix endpoint

crates/core/vsock_gateway_runtime/src/
├── lib.rs                           # C4 Gateway Runtime facade
├── config.rs                        # queue、quota、timeouts 与 stacks
├── runtime.rs                       # accept/runtime lifecycle
├── session.rs                       # SB connection、sb-id 与 per-SB quota
└── upstream.rs                      # 单 TCP upstream、gateway-id、Heartbeat 与重连

crates/core/gateway_ingest_runtime/src/
├── lib.rs                           # C4 Gateway Ingest facade
├── runtime.rs                       # gateway connection registry 与 frame delivery
├── sink.rs                          # sandbox observation sink port
└── status.rs                        # gateway ingest counters

crates/apps/daemon/src/
├── startup/bootstrap.rs             # Hand listener、Evidence/Alert stores 与 daemon 生命周期接线
└── services/
    ├── sandbox_plugins/
    │   ├── mod.rs                   # sandbox plugin services facade
    │   ├── manager.rs               # selector registry 与 consumer lifecycle
    │   └── route_sink.rs            # C4 plugin-or-Evidence 互斥路由
    └── sandbox_alerts/
        ├── mod.rs                   # Sandbox Alert service facade
        └── forwarder.rs             # committed alert 的外发边界
```

## 6. 当前部署与验证资产

```text
deploy/execution-isolation/
├── actrail-sb.toml                              # Guest daemon checked-in profile
├── actrail-vsock-gateway.toml                   # Firecracker 主线 gateway profile
├── actraild-sandbox-resource-alert.startup.toml # actraild 执行隔离启动片段
└── README.md                                    # 生成、安装、启动与 endpoint 关系

tests/v2/
├── common/                                      # release binary、配置刷新与共享编排
└── regression/                                  # 唯一允许的端到端/回归场景
```
