# eBPF 源码布局

主 eBPF collector 按基础定义、wire ABI、共享运行时和观测领域组织源码。目录边界表达状态与行为的所有权；文件拆分不改变 BPF program、attach point、map ABI 或编译后的 translation unit。

```text
bpf/
├── live_observation.bpf.c
├── common/
│   ├── constants.h
│   ├── helpers.h
│   ├── kernel_types.h
│   └── uprobe_registers.h
├── abi/
│   ├── observation.h
│   ├── process.h
│   ├── network.h
│   ├── fd_io.h
│   ├── file_path.h
│   └── payload.h
├── runtime/
│   ├── event_transport.h
│   ├── process_identity.h
│   ├── trace_membership.h
│   ├── process_generation.h
│   └── endpoint.h
├── launch_binding/
│   ├── binding.h
│   └── impl/
│       ├── task_storage.h
│       └── pid_generation_hash.h
├── process/
│   ├── state.h
│   ├── observe.h
│   └── programs.h
├── fd/
│   ├── types.h
│   ├── maps.h
│   ├── descriptor.h
│   ├── index.h
│   ├── lifecycle.h
│   ├── sweep.h
│   ├── suppressed.h
│   └── programs.h
├── network/
│   ├── state.h
│   ├── observe.h
│   └── programs.h
├── file/
│   ├── state.h
│   ├── paths.h
│   ├── open.h
│   ├── bulk_read.h
│   ├── observe.h
│   └── programs.h
├── payload/
│   ├── socket_types.h
│   ├── socket_state.h
│   ├── socket_tls.h
│   ├── socket_capture.h
│   ├── stdio_capture.h
│   └── programs.h
└── tls/
    ├── state.h
    ├── capture.h
    ├── completion.h
    ├── diagnostics.h
    ├── rustls.h
    └── programs.h
```

## 分层职责

`common/` 只包含无观测领域语义的常量、内核类型声明、寄存器访问和基础工具。它不拥有 map，不查询 trace membership，也不构造事件。

`abi/` 定义内核到用户态传输的 wire record。公共 observation header、进程、网络、FD I/O、文件路径和 payload record 按事件 family 分开。ABI 文件只描述枚举、字段和布局，不包含 map 操作或事件构造逻辑。record 语义及 PID 坐标由[eBPF 事件 ABI](ebpf-event-abi.md)定义。

`runtime/` 提供多个观测领域共同依赖的运行时机制：事件 transport、PID 身份投影、trace membership、进程代际和 endpoint 读取。共享 runtime 不依赖 FD、network、file、payload 或 TLS 等领域模块。

`launch_binding/` 拥有启动绑定协议及按内核能力选择的 task-storage 和 PID-generation 实现。两种实现向 process 领域提供相同的绑定行为，不把实现差异传播到其他领域。

每个观测领域拥有自己的 map、pending state、事件构造与 BPF program 入口。`state.h` 或 `maps.h` 保存该领域状态；`observe.h` 和 `capture.h` 负责从内核上下文构造 typed record；`programs.h` 只保留 `SEC(...)` 入口和薄参数适配。FD 领域同时作为 file、network 和 payload 使用的受控基础服务，负责 descriptor 读取、FD slot、kernel file object、引用计数和生命周期。socket 创建暂态保存在 network state 中，由 FD lifecycle 在创建与线程退出路径消费。

## 依赖方向

依赖只沿以下方向下降：

```text
live_observation.bpf.c
          ↓
domain/programs.h
          ↓
domain state and behavior ──→ fd service
          ↓
runtime
          ↓
abi and common
```

底层模块不 include 领域 program 或上层聚合文件。共享状态由明确的领域拥有；FD lifecycle 对 network socket 创建暂态的访问是 socket 注册与现有退出清理路径的集成边界。通用 runtime 不通过 include 把某个领域实现重新引入依赖图。

## 编译边界

`live_observation.bpf.c` 是主 collector 的组合入口，只 include 各领域的 `programs.h` 并声明制品级信息。所有模块仍编译为同一个 BPF translation unit，源码拆分本身不引入 BPF-to-BPF 调用、额外 map 查询或运行时跳转。

需要降低 verifier 复杂度时，直接调整具体 program 的控制流、内联边界或状态访问；不能把移动代码到另一个 header 当作 verifier 优化。反过来，任何性能优化也不得破坏上述所有权和依赖边界。
