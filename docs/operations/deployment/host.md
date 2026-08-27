# 在 Linux 主机部署 AcTrail

> 本文说明如何从 release 产物建立持久、可检查、可回滚的 Linux 主机实例。

## 环境要求

- 满足 [平台支持范围](../../reference/platform-support.md)；
- 运行身份可以访问所选 collector 的内核接口；
- 为 `/run/actrail`、`/var/lib/actrail` 和 `/var/log/actrail` 提供受保护的本地文件系统；
- 至少安装 `actraild`、`actrailctl`、`actrailviewer`；TLS sync 还需要 `libactrail_tls_payload_probe_sync.so` 可被 launch 解析。

## 1. 安装 release

openEuler/RPM 环境从 release page 下载与发行版和架构匹配的包：

```text
https://gitcode.com/openeuler/AcTrail/releases/latest
```

部署人员应将文件名替换为下载的实际 package：

```bash
sudo rpm -Uvh AcTrail-<VERSION>-<RELEASE>.<DISTRO>.<ARCH>.rpm
```

从源码 checkout 安装时，安装脚本会检查 build dependencies、构建 release binary 和 TLS sync runtime，并复制正式产物：

```bash
./scripts/install-release.sh /usr/local/bin
```

### 从源码构建时固定 event transport

默认 release 构建会探测构建主机的 BPF ring buffer capability：能够确认支持时编译 ring buffer，否则编译 perf buffer。需要为目标环境固定 perf buffer 制品时，不依赖自动探测：

```bash
cargo build --release -p daemon --features perf-buffer
```

也可用 `ACTRAIL_EBPF_EVENT_TRANSPORT=perf-buffer` 或 `ring-buffer` 显式选择。构建日志和 daemon 的 `host_ebpf_preflight completed` 诊断都会显示最终 `event_transport`。两种路径的消费与丢失处理见 [eBPF 事件传输](../../architecture/components/ebpf-event-transport.md)。

## 2. 生成配置

系统配置由已经安装的 binary 生成：

```bash
sudo actraild init
```

`/etc/actrail/actraild.conf` 已存在时不会被自动替换。按 [配置 daemon](../daemon/configure.md) 审查 socket、存储、保留、采集、治理和出口设置。

## 3. 做只读主机检查

```bash
uname -m
id -u
test -r /sys/kernel/btf/vmlinux
grep -w tracefs /proc/self/mountinfo
test -w /sys/kernel/tracing || test -w /sys/kernel/debug/tracing
sysctl kernel.perf_event_paranoid kernel.unprivileged_bpf_disabled
```

检查结果的解释见 [部署前检查](../troubleshooting/preflight.md)。支持状态不能仅根据发行版名称判断。

## 4. 启动并检查 readiness

后台模式：

```bash
sudo actraild start
sudo actraild status
sudo actrailctl doctor
```

systemd 等 supervisor 应直接执行：

```bash
/usr/local/bin/actraild --config /etc/actrail/actraild.conf run
```

服务只有在 control socket、存储和必需 collector 都 ready 后才应对外宣称可用。

## 升级与回滚

项目版本达到 `1.0.0` 前不承诺配置或 SQLite schema 向下兼容。升级前，运维人员应停止 daemon，备份当前二进制、配置、SQLite 及 WAL/SHM 文件，并用新版本 `init --output` 生成模板进行人工比较。安装过程不得自动覆盖现有配置或数据库。

回滚时停止新版本，恢复同一版本集合的二进制、配置与完整 SQLite 文件集，再启动并运行 `status` 和 `doctor`。如果新版本已经用不兼容 schema 写入数据库，不能只回滚二进制；必须同时恢复升级前的存储备份。

RPM 升级使用与安装相同的 `rpm -Uvh` 流程，并在升级后重新执行 `status` 与 `doctor`。其他发行版使用源码 release 安装路径。
