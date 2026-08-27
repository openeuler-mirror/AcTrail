# openEuler Kata guest 构建与验证

Kata guest 可以使用 openEuler。宿主机、guest rootfs 和 workload 镜像是三个独立
维度。当前已验证组合包括原有 openEuler 24.03 LTS-SP1 x86_64/cgroup v2 环境，
以及本页记录的 openEuler 24.09 ARM64/cgroup v1 宿主环境；两者均使用 Kata
Containers 3.32.0 和 StratoVirt 2.4.0，但 containerd、guest 内核和构建链不同。

宿主预检同时接受 cgroup v2 和具备 `blkio`、`cpu,cpuacct`、`cpuset`、
`devices`、`freezer`、`hugetlb`、`memory`、`pids` 必要控制器的 cgroup v1。
AcTrail 身份解析器覆盖两种 `/proc/<pid>/cgroup` 格式。ARM64 宿主 cgroup v1 已
完成 KVM/Kata/StratoVirt 完整 E2E；Kata guest 由 3.32 默认内核参数切为 cgroup
v2，因此 guest 内 cgroup v1 身份格式仍由 fixture 覆盖，不能与宿主 cgroup 模式
混为一谈。

本仓库不提交生成的 rootfs image。主路径使用仓库内的
`build-openeuler-image.sh`，在非 privileged 构建容器中直接生成包含 AcTrail 的
候选镜像；已有、版本匹配的 Kata image 也可以通过 `inject-image.sh` 复制后注入。

## 已验证的 ARM64 openEuler 组合

验证目标如下。发行版 Kata 3.2.0 只保留作回退，不参与候选运行路径；Cloud
Hypervisor ARM64 不在验证范围内，不能从 StratoVirt 结果外推。

复现该环境时需要区分以下关键组件，不能只根据发行版或内核版本推断兼容性：

| 组件 | ARM64 验证组合 |
|---|---|
| 宿主 | openEuler 24.09 ARM64，cgroup v1 |
| 容器运行时 | containerd 1.6.22，Kata shim/runtime 3.32.0 |
| VMM | StratoVirt 2.4.0 |
| 接口 Profile | openEuler 24.09 Kata SRPM Linux 6.6.0，增加 `CONFIG_VIRTIO_FS=y` |
| 数据 Profile | Kata 3.32.0 官方 `vmlinux-debug.container`，带 BTF/eBPF/tracefs |
| guest/workload | openEuler 24.03 LTS-SP3 guest、kata-agent 3.32.0、cgroup v2；openEuler 24.09 workload |

openEuler 本身是 Linux 发行版，因此 `file kernel`、启动日志和 `uname` 显示 Linux
内核版本是正常现象，不表示 guest 发行版发生变化。ARM64 openEuler 验收默认镜像是
`actrail-openeuler-workload:24.09`；runner 会读取 `/etc/os-release` 并断言
`ID=openEuler`。

仓库提供 `build-openeuler-image.sh`：在 openEuler 24.03 构建容器中使用
`dnf --installroot` 创建 systemd rootfs，从 Kata 3.32.0 官方 initrd 提取
`kata-agent`、匹配的 systemd unit 和 `default-policy.rego`，再通过
`mkfs.ext4 -d` 封装镜像。该路径不需要 loop mount 或 privileged 容器。

### 安装 Kata 3.32.0 host runtime

官方归档按宿主架构固定，安装器用 `uname -m` 选择接受哪一个：

```text
aarch64:
https://github.com/kata-containers/kata-containers/releases/download/3.32.0/kata-static-3.32.0-arm64.tar.zst
sha256:8736c054d9223974735394f822000823baef509e1c33405ec798240fa9b6e4b5

x86_64:
https://github.com/kata-containers/kata-containers/releases/download/3.32.0/kata-static-3.32.0-amd64.tar.zst
sha256:1449ecea50bd91fa73a94648db195d18950fe869ba4b1f12d05f55f1fa7c1b01
```

把另一架构的归档喂给安装器会在摘要校验处失败，不会装上宿主跑不了的二进制。

目标服务器不能直连 GitHub 时，在可联网机器下载、校验并用 `rsync -P` 续传。然后：

```bash
sudo ./deploy/virtual-container/host/install-kata-3.32.sh \
  --archive "$PWD/local/kata/downloads/kata-static-3.32.0-arm64.tar.zst"

# x86_64 宿主换成对应归档：
sudo ./deploy/virtual-container/host/install-kata-3.32.sh \
  --archive "$PWD/local/kata/downloads/kata-static-3.32.0-amd64.tar.zst"
```

安装器展开到 `/opt/kata-3.32.0`，用 `/opt/kata` 和 `/usr/local/bin` symlink 激活；
不会覆盖发行版拥有的 `/usr/bin/kata-runtime` 或
`/usr/bin/containerd-shim-kata-v2`。版本化 runtime
`io.containerd.kata332.v2` 对应
`/usr/local/bin/containerd-shim-kata332-v2`，可避免 containerd 仍命中旧的 3.2
shim；删除四个激活 symlink 即可回到发行版命令查找路径，3.2 RPM 文件本身保持
不变。

`Containerfile.openEuler` 同时固定 rootfs 和内核构建依赖。制作 24.03 guest
rootfs 时，先使用 24.03 基础镜像创建构建环境：

```bash
docker build \
  --build-arg BASE_IMAGE=openeuler-24.03-lts-sp3:latest \
  -f deploy/virtual-container/guest/Containerfile.openEuler \
  -t actrail-openeuler-builder:24.03 \
  deploy/virtual-container/guest
```

AcTrail release artifacts 仍使用项目固定的 rustup toolchain 编译。先在宿主仓库中
准备 guest bundle 和 Kata reference initrd：

```bash
export KATA_WORK="$PWD/local/kata"

BUNDLE_DIR="$KATA_WORK/guest-bundle" ACTRAIL_BUILD=0 \
  ./tests/v2/regression/virtual_container/prepare-guest-bundle.sh

mkdir -p "$KATA_WORK/kata-3.32.0-reference"
tar --zstd -xf "$KATA_WORK/downloads/kata-static-3.32.0-arm64.tar.zst" \
  -C "$KATA_WORK/kata-3.32.0-reference" \
  ./opt/kata/share/kata-containers/kata-containers-initrd.img \
  ./opt/kata/share/kata-containers/kata-alpine-3.22.initrd

```

再由构建容器内的 root 用户执行镜像生成。仓库挂载在容器内相同的工作目录，输出仍
落在宿主的 `local/kata/`；该过程不需要 privileged 容器或宿主 sudo：

```bash
docker run --rm \
  -v "$PWD:/workspace/AcTrail" \
  -w /workspace/AcTrail \
  actrail-openeuler-builder:24.03 \
  ./deploy/virtual-container/guest/build-openeuler-image.sh \
    --rootfs /workspace/AcTrail/local/kata/openeuler-rootfs-agent332 \
    --output-image /workspace/AcTrail/local/kata/kata-openeuler-actrail-agent332.img \
    --kata-initrd /workspace/AcTrail/local/kata/kata-3.32.0-reference/opt/kata/share/kata-containers/kata-alpine-3.22.initrd \
    --bundle /workspace/AcTrail/local/kata/guest-bundle \
    --expected-agent-version 3.32.0 \
    --require-agent-policy \
    --startup-dependency optional
```

rootfs 输出目录必须不存在或为空，镜像路径必须不存在；脚本不会覆盖旧产物。生成后以官方
Kata 3.32.0 配置分别生成普通接口候选和数据验收候选，同时保持 kernel、VMM、
virtiofsd 和 allowlist 都指向真实可访问路径。先显式设置并检查宿主资产；仓库不会
生成 StratoVirt wrapper 或 virtiofsd：

```bash
export STRATOVIRT_BIN=/absolute/path/to/stratovirt
export VIRTIOFSD_BIN=/absolute/path/to/virtiofsd
test -x "$STRATOVIRT_BIN"
test -x "$VIRTIOFSD_BIN"
```

普通接口候选使用 openEuler 6.6
VirtioFS 内核，不启用 debug console：

```bash
./deploy/virtual-container/host/prepare-stratovirt-config.py \
  --output "$KATA_WORK/configuration-stratovirt-3.32.toml" \
  --hypervisor "$STRATOVIRT_BIN" \
  --kernel "$KATA_WORK/kernel-oe2409-virtiofs" \
  --image "$KATA_WORK/kata-openeuler-actrail-agent332.img" \
  --virtiofsd "$VIRTIOFSD_BIN"
```

数据验收候选使用 Kata 3.32.0 官方 BTF debug 内核、至少两个 vCPU，并只为验收启用
debug console：

```bash
./deploy/virtual-container/host/prepare-stratovirt-config.py \
  --output "$KATA_WORK/configuration-stratovirt-3.32-data.toml" \
  --hypervisor "$STRATOVIRT_BIN" \
  --kernel /opt/kata/share/kata-containers/vmlinux-debug.container \
  --image "$KATA_WORK/kata-openeuler-actrail-agent332.img" \
  --virtiofsd "$VIRTIOFSD_BIN" \
  --default-vcpus 2 \
  --debug

./tests/v2/regression/virtual_container/validate-runtime-config.py \
  --backend stratovirt \
  --require-kernel-config \
  "$KATA_WORK/configuration-stratovirt-3.32.toml"

./tests/v2/regression/virtual_container/validate-runtime-config.py \
  --backend stratovirt \
  --require-kernel-config \
  --require-ebpf \
  "$KATA_WORK/configuration-stratovirt-3.32-data.toml"
```

### openEuler 24.09 ARM64 Kata guest 内核

openEuler 24.09 的 `kata-containers-3.2.0-4.oe2409` ARM64 配置包含
`CONFIG_VSOCKETS=y`、`CONFIG_VIRTIO_VSOCKETS=y`、`CONFIG_VIRTIO_MMIO=y` 和
`CONFIG_FUSE_FS=y`，但没有 `CONFIG_VIRTIO_FS`。该包的 StratoVirt 配置却使用
`shared_fs = "virtio-fs"`。结果是 agent 已经能接收 `create_sandbox`，随后挂载
`kataShared` 时返回：

```text
failed to mount "kataShared" ... ENODEV: No such device
```

这里的 `3.2.0` 只标识 openEuler SRPM 的内核源码/配置来源；生成的 Linux 内核可由
Kata 3.32 runtime 启动。host shim 和 guest agent 仍必须是 3.32.0，不能因为 SRPM
文件名又切回 3.2 runtime。

不能用 openEuler 24.03 构建容器里另一个同名 `6.6.0` 内核代替：实机已观察到该
内核的 guest vsock bind 返回 `EOPNOTSUPP`。仓库提供
`build-openeuler-kata-kernel.sh`，它使用精确的 24.09 Kata SRPM，校验签名、
NEVR 和 SHA256，严格应用 SRPM 中的 openEuler ARM64 配置补丁，只增加
`CONFIG_VIRTIO_FS=y`，并保留最终配置和构建来源。

先载入官方 openEuler 24.09 ARM64 Docker 基础镜像并构建隔离工具环境：

```bash
wget -c \
  https://repo.openeuler.org/openEuler-24.09/docker_img/aarch64/openEuler-docker.aarch64.tar.xz
wget -c \
  https://repo.openeuler.org/openEuler-24.09/docker_img/aarch64/openEuler-docker.aarch64.tar.xz.sha256sum
sha256sum -c openEuler-docker.aarch64.tar.xz.sha256sum
xz -dc openEuler-docker.aarch64.tar.xz | docker load

docker build \
  --build-arg BASE_IMAGE=openeuler-24.09:latest \
  -f deploy/virtual-container/guest/Containerfile.openEuler \
  -t actrail-openeuler-builder:24.09 \
  deploy/virtual-container/guest
```

下载并构建候选内核：

```bash
export KATA_WORK="$PWD/local/kata"

wget -c \
  https://repo.openeuler.org/openEuler-24.09/source/Packages/kata-containers-3.2.0-4.oe2409.src.rpm \
  -O "$KATA_WORK/kata-containers-3.2.0-4.oe2409.src.rpm"
printf '%s  %s\n' \
  261269ab04a524d6c5e34473cf03c82588780dbcb01536bfc7b637de8925bba0 \
  "$KATA_WORK/kata-containers-3.2.0-4.oe2409.src.rpm" |
  sha256sum -c -

docker run --rm \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -v "$PWD:/workspace/AcTrail" \
  -w /workspace/AcTrail \
  actrail-openeuler-builder:24.09 \
  ./deploy/virtual-container/guest/build-openeuler-kata-kernel.sh \
    --source-rpm /workspace/AcTrail/local/kata/kata-containers-3.2.0-4.oe2409.src.rpm \
    --output-kernel /workspace/AcTrail/local/kata/kernel-oe2409-virtiofs \
    --work-dir /workspace/AcTrail/local/kata/kernel-build-oe2409 \
    --jobs 32
```

候选配置使用：

```toml
kernel = "/absolute/path/local/kata/kernel-oe2409-virtiofs"
image = "/absolute/path/local/kata/kata-openeuler-actrail-agent332.img"
shared_fs = "virtio-fs"
```

不要替换宿主 `/var/lib/kata/kernel`，也不要重装系统 RPM。构建结果旁的
`.config` 必须包含：

```text
CONFIG_FUSE_FS=y
CONFIG_VIRTIO_FS=y
CONFIG_VIRTIO_MMIO=y
CONFIG_VSOCKETS=y
CONFIG_VIRTIO_VSOCKETS=y
CONFIG_VIRTIO_VSOCKETS_COMMON=y
```

这个定制 6.6 内核用于接口矩阵和验证 eBPF 不可用时的显式 fail-open；它没有本数据
矩阵要求的完整 `CONFIG_BPF_SYSCALL`、BTF 和 tracing 配置。TLS/eBPF 数据矩阵改用
Kata 3.32.0 官方 `vmlinux-debug.container`。配置校验器会解析该 symlink，按实际
内核名查找同目录的 `config-*`，并检查 VirtioFS、BTF、eBPF 和 tracing 选项。debug
内核是验收资产，不代表生产镜像必须使用 debug 内核。

### openEuler 24.09 workload 镜像

官方精简 openEuler rootfs 不含 `/usr/bin/setpriv`，而 openEuler containerd 1.6
的 `ctr run` 又不支持 `--user`。因此 workload 候选镜像通过
`workload/Containerfile.openEuler` 安装 `util-linux`，保留数字 UID/GID 权限边界。
构建、bundle 和 Kata 接口验证命令统一见
[`../workload/README.md`](../workload/README.md)，避免两处流程发生漂移。

### 依赖与已知环境坑

- 构建容器必须确认为 openEuler 24.03；不能在 Ubuntu 容器中制作后将其记录为
  openEuler guest。内核构建则必须使用 openEuler 24.09 ARM64 容器。
- 构建工具依赖由 `Containerfile.openEuler` 固定，包括 clang/LLVM、ELF/zlib/musl
  开发包、bc、bison、flex、patch、Perl、dtc、cpio 压缩工具、dnf 插件、
  e2fsprogs 和宿主共享目录所需的 virtiofsd。Linux 6.6 构建到
  `lib/oid_registry_data.c` 时需要 Perl；基础容器不一定预装。
- guest installroot 固定安装 systemd、chrony、iptables、iproute、util-linux、
  kmod、libseccomp、OpenSSL、glibc/libgcc 和基本诊断工具。
- 目标环境不能直连 GitHub 时，Kata 3.32.0 官方归档先在可联网机器按固定 SHA256
  校验，再用 `rsync -P` 续传；镜像构建从该归档的同版本 initrd 提取 agent、unit
  和 policy，不在线拉 Kata 源码。
- 3.32 静态发行包安装在版本化 `/opt/kata-3.32.0`，发行版 3.2 RPM 保留在
  `/usr`。测试使用仓库 `local/kata/` 下的用户候选配置，不修改任一
  发行版默认配置。
- guest kernel 首先应检查宿主 Kata RPM 的配套资产。不能把 openEuler 24.03
  构建容器中的 `/var/lib/kata/kernel` 复制给 24.09 宿主的 shim/VMM：即使两者都
  显示 Linux 6.6.0，发行版配置和补丁仍可能不同，尤其会影响 virtio-vsock。
  当前 24.09 RPM 内核虽能使用 vsock，却缺少 StratoVirt 所需的
  `CONFIG_VIRTIO_FS`；ARM64 候选因此改为上述“精确 24.09 SRPM + 单配置增量”
  内核，而不是 24.03 内核或任意上游内核。
- Docker 自带的内部 containerd 不等于系统 containerd。`ctr version` 必须能连接
  `/run/containerd/containerd.sock`；仅有 `ctr` 命令或正在运行 Docker 不能通过
  Kata 验收。openEuler 系统 `containerd.service` 默认可能是 disabled/inactive，
  需要显式启用后再运行 live E2E。
- 3.32 的 `kata-containers-initrd.img` 是官方 reference initrd，不是 openEuler
  guest rootfs。这里只从中提取匹配的 agent、systemd unit 和 policy；AcTrail 与
  openEuler 用户态仍封装到独立 ext4 rootfs image。
- 非 privileged 构建容器不能使用 losetup/mount。`build-openeuler-image.sh` 使用
  `mkfs.ext4 -d`；仅对已有镜像做副本注入时才使用需要 loop mount 的
  `inject-image.sh`。
- 当前 StratoVirt runtime 使用 `root=/dev/vda1`。构建脚本先通过
  `mkfs.ext4 -d` 离线生成文件系统，再用 `sfdisk` 和 `dd` 将它放进 1 MiB
  对齐的第一分区；整个过程仍不需要 mount、loop device 或 privileged 容器。
  候选配置不要额外设置 `root=`，保持 Kata 默认的 `/dev/vda1`。
- StratoVirt 配置默认使用 `shared_fs = "virtio-fs"`，因此仅有 VMM 二进制仍不能
  启动 workload。候选配置的 `virtio_fs_daemon` 和
  `valid_virtio_fs_daemon_paths` 必须同时指向宿主可执行的同一个 virtiofsd，
  guest 内核还必须内建 `CONFIG_FUSE_FS=y` 与 `CONFIG_VIRTIO_FS=y`。
- guest 启动后 `/dev` 是运行时挂载的 devtmpfs；制作 ext4 镜像时预先创建的
  `/dev/actrail` 会被覆盖。openEuler 24.03 的通用
  `systemd-tmpfiles-setup.service` 又明确排除 `/dev`，所以仓库不仅安装
  `actrail-tmpfiles.conf`，还让 `actraild.service` 与
  `10-actrail-workload-interface.conf` 各自在启动前显式执行同一条幂等的
  `systemd-tmpfiles --create --prefix=/dev/actrail`。前者保证 daemon 不会因为
  optional 模式下的并行启动竞态而失败，后者保证 agent 接受容器请求前 bind
  source 一定存在；缺少后者仍可能间歇出现
  `Could not resolve symlink for source /dev/actrail`。
- host runtime、guest agent 和基础配置必须同时固定为 3.32.0。发行版 Kata 3.2
  runtime 对 guest-only `/dev` bind mount 的路径判断过旧，会在宿主解析
  `/dev/actrail` 并报 `Could not resolve symlink`。所需的 guest-local、宿主不存在
  source 行为从 Kata 3.22 首次正式发布；若必须停留在 3.2，需要定向回移该行为并
  重新维护 shim/runtime，而不能通过宿主 `/dev` 伪目录绕过。
- guest eBPF 冷启动可能超过 6 秒。`actraild.service` 使用 60 秒启动超时，数据 runner
  最多轮询 60 秒真实 control socket，并输出 `ACTRAIL_GUEST_CONTROL_READY`；不要用
  固定 `sleep` 判断 daemon 已就绪。
- TLS 测试服务必须在 control socket ready 后单独启动，并用 `-naccept 1` 限定一次
  连接。不能让 TLS server 与 300 秒主任务共用退出时限，否则二者同时结束时 shim
  可能先被清理，只留下 `shim-monitor.sock` 不存在的误报。
- 官方 6.18 debug 内核与 StratoVirt 2.4 在强制清理 sandbox 时可能打印 guest 内
  `Busy inodes after unmount of virtiofs` 警告。当前所有断言都在清理前完成且宿主不
  受影响；生产内核与优雅 shutdown 仍需在打包阶段单独固化。
- Docker 构建上下文不要使用仓库根目录；`local/kata` 可能含 root 所有的 guest
  rootfs。两个 Containerfile 都不复制仓库内容，应分别使用
  `deploy/virtual-container/guest` 或 `deploy/virtual-container/workload` 作为窄
  context。

## 可选：向已有 Kata image 注入

主路径是上面的 `build-openeuler-image.sh`。如果已经有与 host shim、guest agent 和
policy 版本匹配的基础 image，可以使用 `inject-image.sh` 创建副本并注入 AcTrail；
该脚本需要宿主 root/loop mount 权限，且绝不原地修改基础镜像：

```bash
sudo ./deploy/virtual-container/guest/inject-image.sh \
  --source-image kata-openeuler-base.img \
  --output-image kata-openeuler-actrail.img \
  --bundle .actrail-guest-bundle \
  --startup-dependency optional
```

上述命令默认只启用 Guest 本地 SQLite。需要实时 OTLP/HTTP 外送时再追加
`--otel-endpoint "$GUEST_OTEL_ENDPOINT"`；该地址必须从 Guest 可达，且 network 模式下
`127.0.0.1` 不是宿主机。无网络 Guest 使用 endpoint
`http://127.0.0.1:14318/v1/traces` 时还必须追加 `--egress-mode vsock-bridge`。

数据面验收镜像可额外传 `--with-viewer`；生产最小镜像不应包含 viewer。从已安装的
`configuration-stratovirt.toml` 生成候选配置时只替换必要资产路径，不修改 Kata
发行版默认配置或默认镜像。

Kata Agent 3.32.0 的发布二进制启用了 `agent-policy`。若只复制 agent 而漏掉
`default-policy.rego`，systemd 会显示 agent 已启动，但 agent 会在监听 vsock 1024
前退出，宿主最终只看到 `timed out connecting to vsock`。策略文件因此是 guest
rootfs 的必要组成，不是 AcTrail 配置。

## 验收配置

普通内核用于验证无 BTF 时的 fail-open 降级。eBPF 数据面验收配置必须：

- 指向带 BTF 的同版本 Kata guest kernel；
- `default_vcpus >= 2`，避免单 vCPU 下 eBPF 压力使 kata-agent 健康检查超时；
- 仅在 V2 data Profile 中启用 Kata debug console，供 guest-root viewer 读取私有
  SQLite；生产配置和 base Profile 不启用 debug console。

完成内容寻址 artifact 与本机 profile 后，通过公共 V2 入口执行完整接口/data 矩阵：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

统一 V2 case 先在 base VM 中验证 guest-root 服务、只读接口、身份和权限边界，再在
data VM 中验证同一拓扑的 TLS-only、eBPF-only 和 combo。data VM 只把 OpenSSL 及其
库闭包作为只读测试资产挂入 workload，不把 daemon 或 viewer 放进 workload。
详细矩阵、手动复现和排障见
[V2 测试文档](../../../tests/v2/regression/virtual_container/v2/README.zh.md)。

## 已验证范围

上述组合已经验证 openEuler guest 识别、`kata-agent` 与 `actraild` 服务生命周期、
workload 的只读接口、权限和身份，以及无 BTF 内核降级和 BTF 内核上的 TLS/eBPF
采集。稳定的验收条件为：

- preflight 无失败项；
- 接口矩阵 `verify`、`deny`、`launch`、`namespace` 全部为 `PASSED`；
- 数据矩阵 `tls-only`、`ebpf-only`、`combo` 全部为 `PASSED`；
- `combo` trace 为 `Completed/Clean`、`diagnostics=0`，事件与网络事件非零，并同时
  命中 OpenSSL `SSL_write` 与 `SSL_read`。

事件数会随内核和调度变化，不作为固定 golden 值；验收断言是事件非零、TLS 双向
marker 命中、trace clean。Cloud Hypervisor ARM64 不在该验证组合中，因此不在本次
覆盖范围。验证结论也不等于签名镜像、打包交付、升级回滚或 Kubernetes
RuntimeClass 已经交付。
