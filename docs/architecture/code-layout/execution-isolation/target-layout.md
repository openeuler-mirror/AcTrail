# AcTrail 执行隔离目标代码布局

本文定义执行隔离代码的稳定物理布局。依赖、状态、性能和故障约束见 [代码布局设计约束](design-constraints.md)。

## 1. Container composition roots

### 1.1 actrail-sb

```text
crates/apps/sb/
├── Cargo.toml                       # C4 Container actrail-sb 的最小组装依赖
└── src/
    ├── lib.rs                       # Container facade；最小导出 CLI、bootstrap、process 与 config
    ├── bin/actrail-sb.rs            # 同一 binary 入口
    ├── cli/
    │   ├── mod.rs                   # C4 Sandbox Control CLI facade
    │   ├── entry.rs                 # daemon/connect/init 调度与 daemon event loop
    │   ├── command.rs               # 静态 config path 与运行时 control socket/CID/port 参数
    │   └── client.rs                # C4 Guest-local Control Client 组装
    └── daemon/
        ├── mod.rs                   # C4 Sandbox Agent Daemon composition facade
        ├── config.rs                # TOML sections、默认 profile、校验与 init 写入
        ├── bootstrap.rs             # collector/sampler/runtime/transport/control server 组装
        ├── instance_lock.rs         # Guest 单 daemon flock owner
        ├── lifecycle.rs             # signalfd、control health fd 与 diagnostics deadline event owner
        └── output.rs                # ready、CLI 结果与集中诊断输出
```

### 1.2 actrail-vsock-gateway

```text
crates/apps/vsock_gateway/
├── Cargo.toml                       # C4 Container gateway 的组装依赖
└── src/
    ├── lib.rs                       # GatewayConfig 与 GatewayBootstrap facade
    ├── bin/actrail-vsock-gateway.rs # gateway binary 入口
    └── startup/
        ├── mod.rs                   # startup facade
        ├── config.rs                # listener/session/upstream/limits 配置
        ├── bootstrap.rs             # Gateway Runtime 与 Guest link listener 组装
        └── backend/
            ├── mod.rs               # backend 解析与 factory
            ├── firecracker.rs       # 主线 uds_path + port
            ├── native.rs            # 可选 Host AF_VSOCK（含 StratoVirt）
            └── cloud_hypervisor.rs  # 可选完整 Unix socket path
```

### 1.3 actraild execution-isolation wiring

```text
crates/apps/daemon/src/
├── startup/bootstrap.rs             # Hand TCP、Sandbox Evidence/Alert stores 的启动与关闭
└── services/
    ├── sandbox_plugins/
    │   ├── mod.rs                   # C4 Sandbox Plugin Delivery facade
    │   ├── configuration.rs         # schema校验、配置文档与可回滚原子文件替换
    │   ├── manager.rs               # selector snapshot、consumer owners与Web在线配置桥接
    │   └── route_sink.rs            # C4 Gateway Ingest 的 plugin-or-Evidence sink
    └── sandbox_alerts/
        ├── mod.rs                   # Sandbox Alert service facade
        └── forwarder.rs             # committed alert forwarding adapter
```

## 2. actrail-sb runtime components

### 2.1 Sandbox Agent Daemon、Connection Gate 与 Session Owner

```text
crates/core/sandbox_agent_runtime/src/
├── lib.rs                           # C4 Sandbox Agent Runtime facade
├── config.rs                        # runtime durations、capacities、batch、stack 与 metrics switch
├── ports.rs                         # collector、sampler、connection 与 transport ports
├── status.rs                        # optional atomic metrics snapshot
├── daemon/
│   ├── mod.rs                       # C4 Daemon Owner facade
│   ├── owner.rs                     # source 预热、预分配、workers、control handle 与 shutdown owner
│   ├── control.rs                   # Connect admission、Busy、daemon control timeout 与 status
│   └── workers.rs                   # I/O/resource workers、baseline request 与 join owner
├── delivery/
│   └── mod.rs                       # C4 Connection Gate：generation、bounded queue 与 delivery outcomes
└── session/
    ├── mod.rs                       # C4 Sandbox Link Session facade
    ├── owner.rs                     # endpoint、active/reconnect state、batch、gate 与 command owner
    ├── protocol.rs                  # SbHello/SbWelcome、ObservationBatch 与 Heartbeat
    ├── status.rs                    # daemon/session/publication consistent snapshot
    └── wake.rs                      # coalesced command/delivery wake token
```

### 2.2 Guest Linux collectors

```text
crates/adapters/collectors/sandbox_linux/
├── Cargo.toml                       # Guest-only libbpf/procfs adapter package
├── build.rs                         # sandbox BPF object build
├── bpf/
│   ├── sandbox_io.bpf.c             # C4 Process I/O/OOM Collector kernel programs、aggregate与event queue
│   └── sandbox_bpf_helpers.h        # BPF helpers
└── src/
    ├── lib.rs                       # collector/resource facade
    ├── config.rs                    # root names、procfs、refresh 与I/O/OOM map capacities
    ├── collector.rs                 # process-I/O与OOM event cycles、combined collector owner
    ├── ebpf.rs                      # lineage、aggregate baseline、OOM queue drain与kernel diagnostics
    ├── procfs.rs                    # boot/process discovery
    ├── resource.rs                  # C4 Guest Resource Sampler
    └── error.rs                     # adapter errors
```

### 2.3 Shared Collector Attachment Adapter

该adapter不是独立C4 Component。

它为Brain侧eBPF Collector和Guest侧Process I/O Collector提供相同的标准tracepoint挂载机制。

```text
crates/adapters/collectors/libbpf_tracepoint/
├── Cargo.toml                       # 共享的轻量libbpf attach adapter
└── src/
    ├── lib.rs                       # 最小facade
    ├── attacher.rs                  # 标准tracepoint分类、tracefs、perf event与ioctl attach
    └── error.rs                     # attach阶段错误
```

## 3. Guest-local control boundary

```text
crates/contracts/sandbox_control/src/
├── lib.rs                           # C4 Sandbox Control contract facade
├── endpoint.rs                      # runtime host CID/port
├── command.rs                       # Connect command
├── response.rs                      # success/rejection codes
├── status.rs                        # daemon/session/publication status
└── port.rs                          # SandboxControlPort

crates/adapters/sandbox_control/uds/src/
├── lib.rs                           # C4 Control Client/Server UDS facade
├── client.rs                        # one-request CLI client
├── server.rs                        # listener bind、health fd、stop/join handle
├── runtime.rs                       # nonblocking poll owner 与 accepted connection set
├── dispatcher.rs                    # nonblocking single-worker asynchronous service dispatch
├── connection.rs                    # bounded request/response phases 与 connection deadline
├── codec.rs                         # bounded local binary codec
└── error.rs                         # UDS/control adapter errors
```

## 4. Observation and link contracts

```text
crates/contracts/sandbox_observation/src/
├── lib.rs                           # C4 SB observation facade
├── observation.rs                   # Observation enum 与 batch
├── process.rs                       # ProcessMarker 与 ProcessIoCounters
├── resource.rs                      # GuestResourceSnapshot
└── oom.rs                           # OOM victim身份、三态归因与可选谱系根

crates/plugins/sandbox_resource_alert/src/
├── lib.rs                           # C4 Sandbox Resource Alert facade
├── config.rs                        # typed动态阈值与静态来源状态容量
├── plugin.rs                        # immutable配置快照、判定与alert store admission
└── state.rs                         # source-scoped CPU 与 memory 越阈状态

crates/contracts/sandbox_link/vsock/src/
├── lib.rs                           # C4 SB↔gateway link facade
├── frame.rs                         # SbHello/SbWelcome/Heartbeat/ObservationBatch
├── stream.rs                        # bounded frame stream
├── batch_codec.rs                   # compact observation codec
└── error.rs                         # protocol errors

crates/contracts/sandbox_link/upstream/src/
├── lib.rs                           # C4 gateway↔actraild link facade
├── frame.rs                         # GatewayHello/Welcome/Heartbeat/ForwardedSbFrame
├── stream.rs                        # bounded frame stream
└── error.rs                         # protocol errors
```

## 5. Transport adapters and Host runtimes

```text
crates/adapters/sandbox_link/vsock/src/
├── lib.rs                           # C4 VSOCK transport facade
├── client.rs                        # actrail-sb transport factory
├── connection.rs                    # stream owner
├── listener.rs                      # gateway listener facade
├── kernel_vsock.rs                  # native AF_VSOCK
└── unix_stream.rs                   # VMM Unix endpoints

crates/core/vsock_gateway_runtime/src/
├── lib.rs                           # C4 Gateway Runtime facade
├── config.rs                        # runtime capacities/timeouts/stacks
├── runtime.rs                       # listener/session/upstream lifecycle
├── session.rs                       # sb-id、SB connection 与 quota
└── upstream.rs                      # gateway-id、TCP sender、Heartbeat 与 reconnect

crates/adapters/sandbox_link/upstream/src/
├── lib.rs                           # C4 Hand TCP adapter facade
├── config.rs                        # listener/connection limits
├── server.rs                        # accept owner
├── connection.rs                    # Gateway protocol connection owner
├── status.rs                        # connection counters
└── error.rs                         # upstream errors

crates/core/gateway_ingest_runtime/src/
├── lib.rs                           # C4 Gateway Ingest facade
├── runtime.rs                       # gateway registration、IDs 与 delivery owner
├── sink.rs                          # observation sink port
└── status.rs                        # runtime snapshot
```

## 6. Plugin and independent storage boundaries

```text
crates/contracts/sandbox_plugin_delivery/src/
├── lib.rs                           # C4 Sandbox Plugin Delivery facade
├── source.rs                        # gateway-id/sb-id source
├── descriptor.rs                    # selector descriptors
├── route_plan.rs                    # immutable route plan
├── matcher.rs                       # intent matcher port
├── publisher.rs                     # plugin publisher port
└── result.rs                        # delivery outcomes

crates/contracts/sandbox_evidence_store/src/
├── lib.rs                           # C4 Sandbox Evidence Store facade
├── record.rs                        # independent evidence records
├── write.rs                         # nonblocking write port
├── read.rs                          # read port
├── lifecycle.rs                     # store lifecycle port
├── status.rs                        # health/status
└── result.rs                        # admission/write outcomes

crates/adapters/storage/sandbox/src/
├── lib.rs                           # C4 Sandbox Evidence DB facade
├── config.rs                        # DB/queue/transaction/retention config
├── schema.rs                        # independent schema
├── codec.rs                         # observation persistence codec
├── writer.rs                        # bounded asynchronous SQLite writer
├── reader.rs                        # read port adapter
└── status.rs                        # store status
```

## 7. actrail-sb configuration tree

```text
SbDaemonConfig
├── instance_lock_path                               # default /run/actrail/actrail-sb.lock
├── collector
│   ├── root_process_names                           # default [xiaoo, claude]
│   ├── procfs_root                                  # default /proc
│   ├── require_initial_root                         # default false
│   ├── root_refresh_interval_ms                     # default 1000
│   ├── tracked_process_capacity                     # default 16384
│   ├── pending_io_capacity                          # default 32768
│   ├── aggregate_capacity                           # default 4096
│   ├── oom_event_capacity                           # default 256
│   └── poll_interval_ms                             # default 1000
├── sampler
│   └── poll_interval_ms                             # default 1000
├── observation_queue
│   └── capacity                                     # default 1024
├── sender
│   ├── batch_max_observations                       # default 256
│   ├── io_timeout_ms                                # default 1000
│   ├── max_silence_interval_ms                      # default 5000
│   ├── reconnect_interval_ms                        # default 1000
│   └── worker_thread_stack_bytes                    # default 524288
├── control
│   ├── socket_path                                  # default /run/actrail/actrail-sb-control.sock
│   ├── socket_mode_octal                            # default 600
│   ├── request_timeout_ms                           # default 5000
│   ├── accepted_connection_max                      # accepted userspace connection limit (default 8)
│   ├── max_frame_bytes                              # default 1024; protocol minimum 523
│   └── worker_thread_stack_bytes                    # default 262144
└── diagnostics
    └── interval_ms                                  # default 0; disabled

SbConnectInvocation
├── control_socket                                   # required Guest-local absolute path
├── host_cid                                         # required runtime endpoint
├── port                                             # required runtime endpoint
├── request_timeout_ms                               # default 5000
└── max_frame_bytes                                  # default 1024
```

`deploy/execution-isolation/actrail-sb.toml` 保存同一 checked-in profile。Host CID 与 VSOCK port 只存在于 `connect` invocation，不进入 daemon TOML。

## 8. Deployment and V2 validation

```text
deploy/execution-isolation/
├── actrail-sb.toml                              # Guest daemon profile
├── actrail-vsock-gateway.toml                   # gateway profile
├── actraild-sandbox-resource-alert.startup.toml # actraild startup fragment
└── README.md                                    # deployment contract

tests/v2/
├── common/                                      # shared release-binary and Kata orchestration
└── regression/
    ├── sandbox_resource_alert_host/             # no-VMM component-path coverage
    ├── execution_isolation_firecracker/         # real Firecracker + UDS-vsock Guest coverage
    ├── execution_isolation_stratovirt/          # real StratoVirt/Kata + native AF_VSOCK coverage
    └── execution_isolation_cloud_hypervisor/    # optional Cloud Hypervisor Unix endpoint coverage
```

VMM cases share observation, alert, artifact, and lifecycle orchestration where the contracts are
identical. Endpoint selection, boot mechanics, architecture support, and acceptance evidence stay
inside each backend case; a passing backend is not evidence for another backend.
