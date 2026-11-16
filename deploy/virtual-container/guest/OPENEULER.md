# openEuler Guest 镜像

> 完整部署步骤以 [虚机容器部署](../../../docs/operations/deployment/virtual-container.md) 3.5 为准；本页说明该组件的原理、参数和边界。

宿主机、Guest 系统和 workload 容器均为 openEuler 24.03。Guest 镜像由
`build-openeuler-image.sh` 在 openEuler 24.03 宿主机（或容器）上构建，只含
openEuler 基础系统和 Kata agent，不含 AcTrail。生成部署包时，
`prepare-artifacts.py` 调用 `inject-image.sh` 复制该镜像，再把 AcTrail 和所选的
外送方式、外送地址、属性模式注入副本；原镜像保持不变，CNI 和 vsock 两种部署包
使用同一个镜像。Guest 内核使用 Kata 3.32.0 静态包中带 BTF 的 `-debug` 内核。

宿主预检同时接受 cgroup v2 和具备 `blkio`、`cpu,cpuacct`、`cpuset`、
`devices`、`freezer`、`hugetlb`、`memory`、`pids` 必要控制器的 cgroup v1。
AcTrail 身份解析器覆盖两种 `/proc/<pid>/cgroup` 格式。Guest 由 Kata 3.32 默认
内核参数切为 cgroup v2，与宿主的 cgroup 模式无关。

## 构建内容

| 内容 | 来源 |
| --- | --- |
| systemd 根文件系统 | 宿主机 openEuler 软件源，`dnf --installroot` 安装 bash、systemd、socat、iproute、iptables、util-linux、OpenSSL、glibc 等基础包 |
| `kata-agent`、`kata-agent.service`、`kata-containers.target` | Kata 3.32.0 静态包的 `kata-containers-initrd.img`（`kata-alpine-3.22.initrd`）；`default.target` 指向 `kata-containers.target` |
| `/etc/kata-opa/default-policy.rego` | 同一 initrd |
| 磁盘镜像 | `mkfs.ext4 -d` 离线生成文件系统，再用 `sfdisk` 和 `dd` 放进 1 MiB 对齐的第一分区，对应 Kata 默认的 `root=/dev/vda1` |

整个过程不挂载文件系统、不使用 loop 设备，不需要特权容器。socat 总是安装，
vsock 外送只在 vsock 部署包中启用 Guest 转发服务。

## 参数

| 参数 | 说明 |
| --- | --- |
| `--rootfs DIR` | 中间根文件系统目录，必须不存在或为空 |
| `--output-image FILE` | 输出镜像，必须不存在；脚本不覆盖已有产物 |
| `--kata-initrd FILE` | Kata 发行版 initrd，提取 agent、unit 和 policy；只支持 gzip 压缩的 initrd |
| `--expected-agent-version V` | 要求 `kata-agent --version` 包含 V；部署时取 `3.32.0`，与宿主 shim 一致 |
| `--require-agent-policy` | initrd 中没有 `default-policy.rego` 时失败 |
| `--size-mib N` | 第一分区大小，默认 1024 MiB，至少 512；注入 AcTrail 时 `inject-image.sh` 另外扩容 |

构建环境必须是 openEuler 24.03（脚本读取 `/etc/os-release` 检查），需要 `cpio`、
`dnf`、`mkfs.ext4`、`sfdisk` 等命令。

## 注意事项

- **版本一致**：宿主 runtime、Guest agent 和基础配置必须同为 Kata 3.32.0。
  `--expected-agent-version` 在构建时拦截不匹配的 initrd。
- **agent 策略文件**：Kata Agent 3.32.0 发布二进制启用了 `agent-policy`。只复制
  agent 而漏掉 `default-policy.rego` 时，systemd 显示 agent 已启动，但 agent 会在
  监听 vsock 1024 前退出，宿主最终只看到 `timed out connecting to vsock`。策略文件
  是 Guest 根文件系统的必要组成，不是 AcTrail 配置。
- **initrd 只是 agent 来源**：`kata-containers-initrd.img` 是 Kata 官方 reference
  initrd，不是 openEuler 根文件系统；这里只从中提取 agent、unit 和 policy。
- **`/dev/actrail`**：Guest 启动后 `/dev` 是运行时挂载的 devtmpfs，镜像中预先创建
  的 `/dev/actrail` 会被覆盖；openEuler 24.03 的 `systemd-tmpfiles-setup.service`
  又排除 `/dev`。因此 AcTrail 注入时除安装 `actrail-tmpfiles.conf` 外，还让
  `actraild.service` 与 `10-actrail-workload-interface.conf` 各自在启动前执行同一条
  幂等的 `systemd-tmpfiles --create --prefix=/dev/actrail`。前者避免 daemon 在
  optional 模式下因并行启动竞态失败，后者保证 agent 接受容器请求前 bind source
  已存在；缺少后者仍可能间歇出现 `Could not resolve symlink for source /dev/actrail`。
- **VirtioFS**：StratoVirt 配置默认 `shared_fs = "virtio-fs"`，Guest 内核必须内建
  `CONFIG_FUSE_FS=y` 与 `CONFIG_VIRTIO_FS=y`，Kata 3.32.0 的 `-debug` 内核满足。
  配置中的 `virtio_fs_daemon` 和 `valid_virtio_fs_daemon_paths` 必须指向宿主上同一个
  可执行的 virtiofsd。
- **系统 containerd**：Docker 自带的 containerd 不等于系统 containerd。`ctr version`
  必须能连接 `/run/containerd/containerd.sock`；openEuler 的 `containerd.service`
  默认可能未启用。
- **eBPF 冷启动**：Guest 中 eBPF 冷启动可能超过 6 秒。`actraild.service` 使用 60 秒
  启动超时，等待就绪时轮询 control socket，不要用固定 `sleep` 判断。
- **内核版本显示**：`file`、启动日志和 `uname` 显示的是 Linux 内核版本，与 Guest
  发行版无关。
- **强制清理告警**：6.18 debug 内核与 StratoVirt 2.4 在强制清理 sandbox 时可能打印
  Guest 内 `Busy inodes after unmount of virtiofs` 警告，宿主不受影响。

## 回归验收

生成部署包与本机 profile 后，通过公共 V2 入口执行接口和数据矩阵：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

V2 case 先在 base VM 中验证 Guest 服务、只读接口、身份和权限边界，再在 data VM
中验证 TLS-only、eBPF-only 和 combo。data 配置使用带 BTF 的内核、至少两个 vCPU，
只在 data Profile 中启用 Kata debug console。详细矩阵和排障见
[V2 测试文档](../../../tests/v2/regression/virtual_container/v2/README.zh.md)。
