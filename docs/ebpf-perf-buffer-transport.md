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
cargo build -p daemon
```

`ebpf_collector` 的 build script 会按以下顺序检测 ringbuf 支持：

1. `bpftool feature probe kernel unprivileged` 报告 ringbuf map/helper 可用。
2. `/sys/kernel/btf/vmlinux` 包含 ringbuf map/helper 符号。
3. `/proc/sys/kernel/osrelease` 或 `uname -r` 显示内核版本不低于 5.8。

只要确认 ringbuf 可用，默认构建就使用 ringbuf；如果检测结果明确不可用，或所有检测方式
都无法确认可用，则自动切到 perfbuf。构建日志会输出最终选择，例如：

```text
AcTrail eBPF event transport: ring-buffer (bpftool reported ringbuf map and helpers)
```

强制 perfbuf 构建：

```bash
cargo build -p daemon --features perf-buffer
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

`loader/object.rs` 新增 `EventBuffer`：

- 选择 ringbuf 时包装 `libbpf_rs::RingBuffer`。
- 自动降级 perfbuf 或 `perf-buffer` feature 强制 perfbuf 时包装 `libbpf_rs::PerfBuffer`。
- perfbuf 注册 `sample_cb` 收集 raw event，注册 `lost_cb` 统计 perf lost count。
- perfbuf page 数由既有 `event_ring_buffer_max_bytes` 换算为 2 的幂 page count。
- perf event array 的 `max_entries` 使用系统 CPU 数量。

`loader.rs` 在每次 `poll_events()` 后检查：

- perf lost count
- `event_transport_diagnostics.reserve_fail`
- `event_transport_diagnostics.output_fail`
- `event_transport_diagnostics.output_fail_bytes`

任何非零值都会返回 `event_transport_loss`，避免生成不完整 trace。

## TLS Payload 说明

TLS direct-copy 的 ABI 最大可达 4MB。perfbuf 路径不能像 ringbuf 一样直接 reserve
这类大事件，否则会引入过大的 per-cpu scratch buffer 并影响加载稳定性。

因此 perfbuf 路径下 TLS direct-copy 会返回未命中，让现有
`bpf-copy-seccomp-fallback` 或 `seccomp-user-read` 路径完成用户态读取。这样不会静默截断
大 payload。
