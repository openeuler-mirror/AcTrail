# Firecracker / Kata 执行隔离真实 xiaoO 测例

## 快速运行

完成下文的宿主前置条件并构建当前 checkout 的 release 后，从仓库根目录
先生成 Firecracker 本机 profile：

```bash
ACTRAIL_HOST_PATH="$HOME/.cargo/bin:$HOME/.local/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/bin:/usr/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/sbin:/usr/sbin:/sbin:/bin"

sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 \
  deploy/virtual-container/host/prepare-v2-test-artifacts.py \
  --backend firecracker \
  --data-kernel /absolute/path/to/bootable-debug-kernel \
  --with-sandbox-observer \
  --xiaoo /absolute/path/to/xiaoo \
  --workload-image-archive /absolute/path/to/workload.docker.tar \
  --write-profile "$PWD/local/kata/v2-test-profile-firecracker.json"
```

只有准备器打印 `ACTRAIL_V2_ARTIFACTS_READY` 后才能运行 case。准备失败时 profile
不会更新；继续使用旧 profile 只会产生 release checksum mismatch。

再运行告警用例：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --profile local/kata/v2-test-profile-firecracker.json \
  --case execution_isolation_firecracker \
  --color never
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 `CARGO_HOME`、
`RUSTUP_HOME`、`~/.cargo/bin`、`~/.local/bin` 和现有 `PATH`。它不会生成
profile。

`local/kata/` 是本机且被 Git 忽略的目录。profile 和 artifact 绑定生成它们的
机器与 checkout，不得从其他机器或其他 checkout 复制使用。

该测例不再直接执行 `firecracker --no-api`，也不复制并手工启动 rootfs。它与 Cloud
Hypervisor、StratoVirt 共用同一个 alert 场景，由 containerd 和 Kata 创建 Firecracker
VM，再验证真实 xiaoO 的 Guest 采集链：

```text
Guest system actrail-sb → Firecracker hybrid VSOCK → Host gateway → actraild
```

Firecracker 的 Kata hybrid-VSOCK base socket 位于
`/run/vc/firecracker/<sandbox>/root/kata.hvsock`。测例只接受 VM 启动后本轮新增的一个
socket，并把 base UDS 与 port `43182` 交给 Firecracker gateway adapter。

场景与另两个 VMM 验证相同的五类告警、独立 SQLite 记录和公开订阅投递。由于
Firecracker 不提供 virtio-fs，测试资产会在 VM 启动后通过受校验的 `ctr tasks exec`
输入送进 workload；真实 xiaoO 已在 artifact 准备阶段写入派生 workload image，运行时
stdin 只传小型脚本、配置和 manifest，不会把 200+ MiB 二进制重复传入。测试也不会把
Host 目录作为普通共享文件系统挂载。workload 容器仍保持非特权，eBPF observer 由
artifact 注入的 Guest systemd 服务运行。

## 准备

目标 Linux/KVM 主机需要 Kata 3.32 的 `configuration-fc.toml`、Firecracker、可供
Firecracker 使用的 Guest kernel/image，以及 containerd devmapper snapshotter。先从
同一 checkout 构建 release，再制备 Firecracker 内容寻址 artifact；真实 xiaoO 路径
必须显式提供，alert 场景还必须注入 Guest observer。快速运行第一步的
`prepare-v2-test-artifacts.py` 命令会生成内容寻址 artifact 和本机 profile。

artifact 输出位于 `local/kata/artifacts/<digest>/`，输入未变化时会命中缓存，不需要在
每次 `test_all` 前重复生成。输入必须是 containerd `ctr images export` 生成的单平台
combined Docker/OCI archive，保留 `manifest.json` 和 content-addressed `LayerSources`，
且所有 layer 都是未压缩 OCI tar；旧式 `<layer-id>/layer.tar` 的 `docker save` archive
不受支持。例如：

```bash
sudo ctr -n default images export \
  --platform "linux/$(uname -m | sed 's/aarch64/arm64/;s/x86_64/amd64/')" \
  /absolute/path/to/workload.docker.tar \
  docker.io/library/actrail-openeuler-workload:24.09
```

archive 的 Docker/OCI 镜像身份与 platform 必须一致。devmapper 是当前 Kata 3.32
Firecracker 无共享文件系统配置下采用的块 rootfs 路径，并非 Firecracker VMM 本身的
固有要求。

## 直接运行公共 `test_all.py`（仅调试）

如需绕过推荐 wrapper，复用本页快速运行开头定义的 `ACTRAIL_HOST_PATH`，
并显式将它传给 sudo：

```bash
sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 tests/v2/regression/test_all.py \
  --profile local/kata/v2-test-profile-firecracker.json \
  --case execution_isolation_firecracker
```

manifest、派生 workload image/archive、runtime config、Guest observer 或 xiaoO
摘要不一致属于部署错误并返回 `FAILED`；缺少 KVM、Kata/containerd、devmapper 或
Firecracker 属于外部条件并返回 `SKIPPED`。

xgovernor 不在本测例内启动。测试 subscriber 使用与 xgovernor 相同的公开告警出口，
用于证明 Firecracker/Kata 的采集和订阅链已经成立。
