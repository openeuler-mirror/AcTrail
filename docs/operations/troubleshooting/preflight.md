# 部署前检查

> 本文说明如何在启动 daemon 前检查目标主机的架构、内核接口、权限和构建依赖。

## 只读检查

统一检查器应从仓库根目录运行：

```bash
python3 scripts/preflight/platform_preflight.py --color always
```

默认从 `target/release` 读取制品。检查其他 release 目录或其中任一制品时，使用 `--bin-dir`；其余制品会从同目录和 `PATH` 解析：

```bash
python3 scripts/preflight/platform_preflight.py --bin-dir /opt/actrail/bin
```

检查器只读取主机状态、release 制品和工具链信息，不加载 eBPF，也不启动 daemon 或工作负载。以下命令可用于逐项复核主机条件：

```bash
uname -m
id -u
test -r /sys/kernel/btf/vmlinux
grep -w tracefs /proc/self/mountinfo
test -w /sys/kernel/tracing || test -w /sys/kernel/debug/tracing
sysctl kernel.perf_event_paranoid kernel.unprivileged_bpf_disabled
```

判断方式：

- `uname -m` 必须是 `x86_64` 或 `aarch64`；
- BTF 文件必须可读；
- tracefs 必须挂载，并对 daemon 身份可写；
- sysctl 与安全策略必须允许所需 tracepoint/uprobe attach；
- `id -u` 为 `0` 是常见部署方式，但容器或 LSM 仍可能移除所需能力。

只读检查不能证明 TLS、seccomp 或 fanotify 路径可用。完整 probe 需要 daemon 已经启动，并且 release binary 位于 `PATH`；只检查当前 namespace 的 launch 前置条件时可加 `--skip-daemon`。

```bash
sudo actrailctl probe \
  --host-ebpf required \
  --seccomp-notify auto \
  --json
```

`--skip-daemon` 的结果不能替代 daemon 根据配置和主机 collector 状态作出的最终 profile 决策。

## 常见阻塞

| 症状 | 检查 | 处理 |
| --- | --- | --- |
| `unsupported eBPF target architecture` | `uname -m` | 使用受支持架构 |
| `kernel BTF is missing` | BTF 文件是否可读 | 安装或启动启用 BTF 的内核 |
| tracefs missing/not writable | mountinfo 与目录权限 | 按主机策略挂载 tracefs 并授权 daemon |
| `perf_event_open` permission error | sysctl、capabilities、LSM | 调整主机策略；不得通过删除 capability 配置静默降级 |
| fanotify `Operation not permitted` | 是否在受限容器、是否具备 permission event 权限 | 改用满足该治理能力的主机或 VM |

只读检查与 `actrailctl probe` 不能替代特定目标平台上的真实工作负载验证。平台支持声明以 [平台支持范围](../../reference/platform-support.md) 为准；尚未完成真实环境验证的目标必须保持“未验证”状态。
