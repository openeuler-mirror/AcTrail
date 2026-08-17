# eBPF Event Transport: Ring Buffer and Perf Buffer

## 背景

主 eBPF collector 支持两种内核事件传输方式：

- `BPF_MAP_TYPE_RINGBUF` + `bpf_ringbuf_*`
- `BPF_MAP_TYPE_PERF_EVENT_ARRAY` + `bpf_perf_event_output`

部分 5.10 环境缺少 ring buffer 支持，因此默认构建会在编译时检测当前环境是否可用
ringbuf：可用时使用 ringbuf，不可用或无法确认时降级为 perfbuf。同时保留 Cargo
feature，用于在任何环境下强制使用 perfbuf。

## 目标

- 默认构建自动选择 event transport：优先 ringbuf，探测不到 ringbuf 时使用 perfbuf。
- 通过 Cargo feature `perf-buffer` 强制使用 perfbuf，覆盖自动探测结果。
- BPF 侧通过编译宏 `ACTRAIL_EVENT_TRANSPORT_PERF` 选择 perfbuf map 和提交 helper。
- Rust 侧通过内部 cfg `actrail_event_transport_perf` 选择 `PerfBuffer` 或 `RingBuffer`。
- 传输丢失不能静默发生：perf lost callback、BPF reserve/output 失败都要被检测，并让 collector 返回错误。

## 使用方式

默认自动选择：

```bash
cargo build --release -p daemon
```

`ebpf_collector` 的 build script 会按以下顺序检测 ringbuf 支持：

1. `bpftool feature probe kernel` 在具备所需权限时报告 ringbuf map/helper 可用或明确
   不可用。
2. `bpftool feature probe kernel unprivileged` 报告 ringbuf map/helper 可用。非特权探测
   的 unavailable 只表示当前权限视角无法确认，不能作为内核不支持的依据。
3. `/sys/kernel/btf/vmlinux` 包含 ringbuf map/helper 符号。
4. `/proc/sys/kernel/osrelease` 或 `uname -r` 显示内核版本；低于 5.8 可确认上游内核
   不支持，5.8 及以上只作为允许条件，不能代替实际 capability 证据。

每个无法确认的探测都会继续到下一种检测方式。只有实际 capability 证据确认 ringbuf
可用时，默认构建才使用 ringbuf；如果特权探测或内核版本确认不可用，或所有检测方式都
无法确认可用，则自动切到 perfbuf。构建日志会输出最终选择，例如：

```text
AcTrail eBPF event transport: ring-buffer (privileged bpftool reported ringbuf map and helpers)
```

daemon 启动时也会在 `host_ebpf_preflight completed` 诊断中记录编译进二进制的
`event_transport`，用于核对实际运行的制品。

强制 perfbuf 构建：

```bash
cargo build --release -p daemon --features perf-buffer
```

`daemon` 的 `perf-buffer` feature 会转发到 `ebpf_collector/perf-buffer`。
`ebpf_collector` build script 在自动选择 perfbuf 或 feature 强制 perfbuf 时，会向 clang
注入：

```text
-DACTRAIL_EVENT_TRANSPORT_PERF
```

同时会给 Rust 侧注入：

```text
--cfg actrail_event_transport_perf
```

## 实现方案

### BPF 侧

`actrail_runtime.h` 提供统一事件传输 wrapper：

- `actrail_event_reserve(size)`
- `actrail_event_submit(ctx, event)`
- `actrail_event_discard(event)`
- `emit_event(ctx, event)`

ringbuf 路径：

- `events` map 类型为 `BPF_MAP_TYPE_RINGBUF`。
- reserve/submit/discard 使用 `bpf_ringbuf_*`。
- 小固定事件继续可用 `bpf_ringbuf_output`。

perfbuf 路径：

- `events` map 类型为 `BPF_MAP_TYPE_PERF_EVENT_ARRAY`。
- `event_scratch` 是 per-cpu array，用作可变大小事件的临时 buffer。
- 提交使用 `bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, data, size)`。
- `event_transport_diagnostics` 记录 reserve/output 失败。

需要 perfbuf 的原因是 `bpf_perf_event_output` 必须拿到当前 BPF program 的 `ctx`，
因此 emit 链路中的 helper 都显式传递 `ctx`。TLS helper 为避免 BPF-to-BPF 调用参数
过多，使用参数结构和宏封装调用。

### Rust 侧

`loader/object.rs` 提供 `EventBuffer`：

- 选择 ringbuf 时包装 `libbpf_rs::RingBuffer`。
- 自动降级 perfbuf 或 `perf-buffer` feature 强制 perfbuf 时包装 `libbpf_rs::PerfBuffer`。
- perfbuf 注册 `sample_cb` 收集 raw event，注册 `lost_cb` 统计 perf lost count。
- perfbuf page 数由既有 `event_ring_buffer_max_bytes` 换算为 2 的幂 page count。
- perf event array 的 `max_entries` 使用系统 CPU 数量。
- `build_with_sink` 允许调用方提供 sample 回调，不强制 `Rc` 内部可变性，
  因此 buffer 可以整体移入专用消费线程。

`loader/consumer.rs` 提供专用消费线程 `EventConsumer`。ringbuf/perfbuf 都是
MPSC，内核 buffer 必须且只能被一个线程 drain，daemon 的重处理（解码、语义投影、
SQLite 持久化）不能再阻塞内核侧消费：

- `EbpfRuntime::from_object` 把 `EventBuffer` 移入 `actrail-ebpf-event-consumer`
  线程；该线程 `ppoll` 内核 buffer 的 epoll fd，每次就绪或 250ms 看门狗超时后
  贪婪 `consume()`，把 raw event 按 4096 条 / 2MiB 切为有界 batch，经容量 32 的
  `sync_channel` 交给主循环。队列满时消费者阻塞，让内核 ring buffer 继续吸收突发。
- 每条消息携带当前 perf lost 总数；raw 为空但 perf lost 变化时也会发送一条仅含
  丢失计数的消息，保证丢包不被静默吞掉。
- 每次入队后通过 eventfd 唤醒 daemon 的 `ppoll` 主循环；主循环 drain 开始时先
  重置 eventfd 再取消息，避免"先取后清"竞态漏唤醒。
- `EbpfRuntime::event_poll_fd()` 现在返回这个 wake eventfd，而不是内核 buffer 的
  epoll fd；runtime 析构时先停消费线程（drop receiver + 写 shutdown eventfd +
  join），再释放 BPF object。

`loader.rs` 的 `poll_events()` / `flush_transport()` 从队列拉取 raw batch：

- `poll_events()` 拉取后 decode + 因果排序；`flush_transport()` 只拉取不解码，
  供一轮重处理结束后再次清空队列，缩短饥饿窗口。
- 每次 drain 后检查 perf lost 总数与 `event_transport_diagnostics` 的
  `reserve_fail`、`output_fail`、`output_fail_bytes` 等计数器，任何非零都会生成
  `event_transport_loss` 诊断并标记 trace 降级，避免生成不完整 trace。

## TLS Payload 说明

TLS direct-copy 的 ABI 最大可达 4MB。perfbuf 路径不能像 ringbuf 一样直接 reserve
这类大事件，否则会引入过大的 per-cpu scratch buffer 并影响加载稳定性。

因此 perfbuf 路径下 TLS direct-copy 会返回未命中，让现有
`bpf-copy-seccomp-fallback` 或 `seccomp-user-read` 路径完成用户态读取。这样不会静默截断
大 payload。
