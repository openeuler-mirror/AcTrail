# ADR 0002：使用 pidfd 完成 launch trace 注册

> 本文记录 launch 注册采用 pidfd、pre-exec barrier 和匹配 userspace/eBPF adapter 的架构决策。

Status: accepted

## 背景

数字 PID 可能在创建和注册之间被复用。pidfd 是 Linux 对某个具体进程的稳定 handle。launch tracing 必须在首次 exec 前把新子进程绑定到 trace，且不能与 exec 竞争或误绑其他进程。

## 决策

创建子进程时原子取得 pidfd，并将其保持在 pre-exec barrier。把 pidfd 传给 daemon，为精确 task 激活 one-shot exec registration，等待 `binding armed` 后再释放子进程。

支持 BPF task storage 时用它把 one-shot trace ID 交给 exec hook。Linux 5.10 因缺少 task storage，使用 host-PID-plus-generation HASH adapter。两种 adapter 保留相同的 pidfd identity、barrier、acknowledgement、promotion 和 `pending_count` contract；每次构建只选择一对匹配的 userspace/eBPF adapter。

仅当 `clone3(CLONE_PIDFD)` 被过滤为 `ENOSYS` 时，允许用 `clone(CLONE_PIDFD | SIGCHLD)` 替代原子创建 syscall；禁止退回数字 PID 注册。

## 后果

- daemon 激活注册前，launcher 不能报告成功跟踪。
- exec 热路径通常只查询 `tracked_traces`，pending one-shot 查询由共享原子计数门控。
- Linux 5.10 需要独立 adapter，并证明 generation 能阻止 PID reuse 混淆。
- 无法取得或激活 pidfd 时中止 launch，而不是静默创建未跟踪进程。
