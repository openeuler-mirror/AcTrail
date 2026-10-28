# 平台支持范围

> 本文说明 AcTrail 支持的主机架构、内核接口和功能权限要求。

## 架构

| 架构 | 支持状态 |
| --- | --- |
| `x86_64` | 支持 |
| `aarch64` / ARM64 | 支持 |
| 32-bit ARM 及其他架构 | 不支持 |

## 所有 live collection 的基础要求

从仓库根目录运行只读检查器，可一次核对下表条件、release 制品与工具链：

```bash
python3 scripts/preflight/platform_preflight.py --color always
```

| 要求 | 用途 | 只读检查 |
| --- | --- | --- |
| root 或等效能力 | 加载与 attach collector | `id -u`，结合 capabilities/LSM/container policy |
| Kernel BTF | eBPF CO-RE | `test -r /sys/kernel/btf/vmlinux` |
| 可写 tracefs control mount | tracepoint attach | `grep -w tracefs /proc/self/mountinfo`，并检查 `/sys/kernel/tracing` 或 `/sys/kernel/debug/tracing` 可写 |
| 允许 perf tracepoint/uprobe | eBPF 与用户态 probe | 检查 `kernel.perf_event_paranoid`、capabilities 和安全策略 |

## 按功能增加的要求

| 功能 | 增加的要求 |
| --- | --- |
| TLS sync 明文 | 目标经 `actrailctl launch` 启动、sync runtime library 可读、resolver 可为实际二进制生成完整 probe plan |
| socket seccomp fallback / exec context | seccomp user notification、`pidfd_open`、`pidfd_getfd`，并允许读取 child 所需内存 |
| 大型用户态 operation 读取 | 对 traced child 的 `process_vm_readv` 权限 |
| fanotify 文件治理 | fanotify permission events 与对应 mount/namespace 权限 |
| Docker workload 的 launch-time seccomp | Docker profile 允许所需 pidfd/seccomp 路径，或显式接受 `auto` 降级 |

## 构建环境

workspace 的最低 Rust 版本是 `1.90`。常用 native build 依赖：

```bash
# openEuler/Fedora/RHEL-like
sudo dnf install -y clang llvm elfutils-devel zlib-devel pkgconf-pkg-config openssl-devel

# Debian/Ubuntu-like
sudo apt-get install -y clang llvm libelf-dev zlib1g-dev pkg-config libssl-dev
```

release build：

```bash
cargo build --release
```

运行环境：与 release 制品兼容的 glibc 和动态库。

构建环境：ABI 版本不高于目标运行环境。
