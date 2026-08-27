# Quickstart：获得第一条本地 trace

> 本文说明如何从源码启动 daemon、运行一个受观测命令，并在命令行中查看第一条 trace。

适用读者：在 Linux 或 Windows Subsystem for Linux（WSL）测试主机上评估 AcTrail 的产品使用者。

Trace 是一次受观测命令及其进程树产生的证据集合。AcTrail 会把低层事件与 payload 关联到同一 trace，并投影为可查询的语义 action。

默认配置会采集进程、文件、网络、标准输入输出以及应用层明文，并可能持久化 prompt、响应、命令行和凭据。该配置仅适用于已获得观测授权的工作负载。

## 前置条件

- `x86_64` 或 `aarch64` Linux/WSL；内核要求见 [平台支持范围](../reference/platform-support.md)。
- Rust `1.90` 或更高版本。
- Clang/LLVM、libelf、zlib、pkg-config、OpenSSL 开发包和 musl build tools。
- Node.js `18` 或更高版本及 npm；后续 dependency installer 会安装 Web frontend dependencies。
- root 或等效内核能力，用于启动 collector 和受观测命令。

第一步是检查并安装仓库声明的 build dependencies：

```bash
./scripts/install-build-deps.sh --install
```

再从仓库根目录构建 release 产物：

```bash
cargo build --release
```

应至少生成 `target/release/actraild`、`actrailctl` 和 `actrailviewer`。

## 1. 生成默认配置

```bash
sudo ./target/release/actraild init
```

命令创建 `/etc/actrail/actraild.conf`。如果文件已经存在，AcTrail 会校验并保留它；只有明确需要替换时才使用 `init --force`。默认运行文件位于 `/run/actrail/`，SQLite 数据位于 `/var/lib/actrail/`，日志位于 `/var/log/actrail/`。

## 2. 启动 daemon

```bash
sudo ./target/release/actraild start
sudo ./target/release/actraild status
sudo ./target/release/actrailctl doctor
```

`status` 应报告 daemon 正在运行，`doctor` 应报告 `storage_ready=true`。启动失败时，运维人员应先查看 `/var/log/actrail/actraild.log`。

## 3. 运行一个受观测命令

```bash
sudo ./target/release/actrailctl launch --name quickstart -- \
  bash -lc 'echo ACTRAIL_QUICKSTART_OK; id >/dev/null; ls /etc/hosts >/dev/null'
```

输出应包含 `trace trace-<N> entered Active` 和 `ACTRAIL_QUICKSTART_OK`。后续查询使用返回的数字 trace ID；子进程树退出后，daemon 会完成该 trace。

## 4. 查看结果

```bash
sudo ./target/release/actrailviewer traces
sudo ./target/release/actrailviewer summary --trace-id <TRACE_ID>
sudo ./target/release/actrailviewer processes --trace-id <TRACE_ID>
sudo ./target/release/actrailviewer actions --trace-id <TRACE_ID>
sudo ./target/release/actrailviewer diagnostics --trace-id <TRACE_ID>
```

查询时，`<TRACE_ID>` 应替换为上一步返回的数字。`traces` 应列出名为 `quickstart` 的记录；`processes` 和 `actions` 应显示该命令产生的进程与语义证据。

## 5. 停止 daemon

```bash
sudo ./target/release/actraild stop
```

`stop` 会等待 trace 收尾和持久化完成。后续日常运行见 [启动与停止](../operations/daemon/start-stop.md)；缩小采集和保留范围前，应先阅读 [安全模型](../concepts/security-model.md) 和 [采集配置](../reference/configuration/collection.md)。
