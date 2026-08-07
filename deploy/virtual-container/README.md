# AcTrail 虚拟容器(Kata)支持范围与部署说明

虚拟容器(Kata Containers)的 sandbox 是一台带独立内核的轻量 VM:宿主 eBPF
探不进 guest,观测必须发生在 guest 内部。目标部署形态是 **guest 内以系统服务
运行 actraild + workload 容器接入 actrailctl/TLS probe**，数据经 `otel-http`
实时推出（guest 随 sandbox 销毁，本地文件不可靠）。

当前 V2 验收通过重复指定 `test_all.py --case` 统一编排：每个 backend 复用
一台 base VM 完成 workload 接口矩阵，再复用一台 data VM 完成 TLS/eBPF 矩阵；
独立并发 case 另启两台 VM 验证双 xiaoO。当前工具生成的是 rootfs 验证镜像，不是
签名发布包，也不适用于 initrd guest。

## Guest Collector endpoint 显式注入

Guest 镜像不能继承 bundle 中的 `COLLECTOR_HOST` 占位地址。构建或复制注入镜像时，
部署方必须通过 `--otel-endpoint` 提供 Guest 网络真实可达的完整 OTLP/HTTP traces
URL，例如 `http://192.0.2.10:4318/v1/traces`。Guest 内的 `127.0.0.1` 指向 Guest
自己，不是宿主机；安装器也会拒绝 loopback、`0.0.0.0`、占位符、query/fragment
和非 `/v1/traces` 路径。

仓库的 [`host-collector/`](host-collector/) 提供受控开发/验收环境使用的主机侧
Collector。其 `OTELCOL_OTLP_HTTP_ENDPOINT` 是主机监听地址，而传入 Guest 的 URL
必须使用 Guest 实际可达的主机地址。明文 HTTP 只适合隔离测试网络，生产环境仍需
HTTPS/mTLS、认证、证书轮换和持久化后端。

## 测试资产的一键准备与运行

### 首次部署：先固定最终 checkout

先把要验收的代码拉到最终部署目录，并从该目录完成后续所有命令：

```bash
git clone <AcTrail 仓库 URL> AcTrail
cd AcTrail
git switch <要验收的分支>
git pull --ff-only
```

`local/kata/` 被 Git 忽略，并且 preparer 生成的 profile、artifact manifest 和二进制
摘要都绑定当前 checkout。它们不会在 Git worktree 之间自动共享。切换到另一个 checkout、
worktree 或目标提交后，必须在新目录重新运行下面的 preparer；不要复制兄弟目录的
`local/kata/v2-test-profile.json`，否则 profile 可能继续引用旧代码、旧二进制或旧镜像。

`run-v2-tests.sh` 默认要求当前 checkout 已存在
`local/kata/v2-test-profile.json`，缺失时会在申请 sudo 和启动 runner 前退出，并指向
本节的准备命令。显式传入 `--profile <path>` 或 `--no-profile` 时不使用该默认检查。

先使用公共 V2 runner 同款 installer 构建并安装当前 release。不要用一次包含多个 package
的 `cargo build` 代替这一步；Cargo feature 集合不同可能生成不同的 TLS sync runtime，
使随后生成的 manifest 在 runner 的 `release_install` 后发生 checksum mismatch：

```bash
sudo -E env \
  "CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}" \
  "RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}" \
  "PATH=$HOME/.local/bin:$HOME/.cargo/bin:$PATH" \
  ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
  bash scripts/install-release.sh
```

installer 成功后，再以同一 VMM backend 的 base/data Kata 配置为 source，生成不可变的
内容寻址 guest image、bundle、runtime config、manifest 和本机 profile。必须保持这个
顺序；公共 V2 runner 会在校验 manifest 前幂等地再次执行同一个 installer。
StratoVirt 使用默认 backend：

```bash
: "${GUEST_OTEL_ENDPOINT:?set a Guest-reachable OTLP/HTTP traces URL}"
sudo -E env "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --otel-endpoint "$GUEST_OTEL_ENDPOINT" \
    --base-config-source /path/to/configuration-base-source.toml \
    --data-config-source /path/to/configuration-data-source.toml \
    --xiaoo /path/to/xiaoo
```

Cloud Hypervisor 使用 Kata 3.32 的 `configuration-clh.toml`，并为 data Profile
提供带 BTF/eBPF 的 guest kernel。准备器只扩展复制后的 Cloud Hypervisor guest
image 和 ext4 rootfs 128 MiB，为 AcTrail bundle 与运行数据保留空间，不修改 source
image：

```bash
sudo -E env "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --backend cloud-hypervisor \
    --otel-endpoint "$GUEST_OTEL_ENDPOINT" \
    --base-config-source /path/to/configuration-clh.toml \
    --data-config-source /path/to/configuration-clh.toml \
    --data-kernel /path/to/vmlinux-debug.container \
    --xiaoo /path/to/xiaoo
```

将 `/path/to/...` 替换为当前机器上的实际绝对路径。

上面的 `GUEST_OTEL_ENDPOINT` 必须由 Kata Guest 实际可达。首次搭建验收环境时，可先按
[`host-collector/README.md`](host-collector/README.md) 启动固定版本的开发 Collector，
再使用 `http://<Guest 可达的主机 IP>:4318/v1/traces`；不要使用 Guest 的 loopback。

source image 只读，所有修改先写 staging，再原子发布到：

```text
local/kata/artifacts/<digest>/manifest.json
```

请从部署用户的 shell 使用上面的 `sudo -E`，不要先切换到 root shell。preparer 只借用
root 权限完成 loop mount 和 containerd 操作；发布完成后会依据 `SUDO_UID`/`SUDO_GID`
把 `local/kata/artifacts/`、本次 artifact 和 `v2-test-profile.json` 的属主还原为调用它的
部署用户。因此无论 checkout 位于哪个目录，这些 checkout-local 文件都应属于发起
`sudo` 的部署用户。

输入摘要不变时直接命中缓存。release、bundle 或 config 发生错配时，V2 会在启动 VM
前失败，不会等到 socket/pidfd 阶段才暴露旧镜像问题。运行两个完整用例：

```bash
deploy/virtual-container/host/run-v2-tests.sh --color never
```

准备和测试不会自动安装/替换宿主 Kata、VMM、内核或系统软件包。宿主一次性安装、
guest 制作和 workload image 构建仍分别由本目录对应工具负责。

## 部署形态与层级

普通 VM 与 Kata 虚拟容器共享同一套 **guest 内采集数据面**,但不是同一个产品
部署形态。Kata 在实现上包含一台轻量 VM,在控制面上再叠加 containerd/CRI、
shim-v2、kata-agent 和容器镜像语义;它不会自动覆盖普通 VM 的 systemd 安装、
云平台生命周期、持久磁盘和实例身份等运维契约。

```text
L3  集群与汇聚        Kubernetes RuntimeClass / metadata / OTEL backend
                         |
L2  部署适配层        +-- L2A 普通容器: namespace/cgroup/容器引擎生命周期
                      +-- L2B 普通 VM: cloud-init/systemd/VM 生命周期
                      `-- L2C Kata: CRI/shim-v2/kata-agent/sandbox 生命周期
                         |
L1  共享 guest 数据面  actraild + actrailctl/TLS probe + BTF/eBPF + OTEL
                         |
L0  虚拟化基础设施     KVM + StratoVirt/Cloud Hypervisor + guest kernel/virtio/vsock
```

这里的 `L0-L3` 是架构层,不是支持成熟度等级。L2A、L2B、L2C 是并行部署适配分支,
不是从普通容器逐级升级到 Kata:

| 维度 | 普通 VM(L2B) | Kata 虚拟容器(L2C) |
|---|---|---|
| 对外管理单位 | 长生命周期 VM | Pod/container sandbox |
| 控制入口 | 云平台/libvirt/直接 VMM + SSH/systemd | Kubernetes/CRI/containerd + Kata |
| AcTrail daemon | VM 内长期系统服务 | 每个 sandbox guest 内随生命周期启动的服务 |
| 身份与归属 | VM/云实例 ID、DMI 或显式 `host.id` | PID + mount namespace 用于授权；Pod/container/host metadata 用于数据归属；runtime ID 仅作 runner 生命周期句柄 |
| 数据落点 | 可使用 VM 持久盘 | sandbox 易失，主路径必须实时外送 |
| 额外适配 | 软件包、升级、systemd、VM shutdown flush | guest 注入、readiness、socket/metadata 契约、sandbox flush |

因此,“Kata 包含 VM”只在 **虚拟化实现和 L1 数据面复用** 上成立,不代表普通 VM
部署已自动获得产品支持。若产品目标只有 Kata,不需要先建设一套完整的普通 VM
产品线;保留一条普通 VM reference baseline 即可,用于更快验证 guest bundle、
systemd、内核能力、OTEL 和 shutdown flush。只有需要对外承诺“AcTrail 可直接部署到
云 VM/传统 VM”时,才把 L2B 单独补齐安装、升级、身份和生命周期验收矩阵。

当前实现状态也应按层区分:普通 VM 可复用已有 host 安装、`host.id` 和 daemon 能力,
但本目录没有普通 VM 专用的 cloud-init、软件包升级和生命周期矩阵;Kata 已有 L1/L2C
能力 e2e,但生产 guest image 注入、RuntimeClass、metadata 和退出 flush 编排仍待完成。

## VMM 适配

| VMM 后端 | 当前定位 | 验证要求 |
|---|---|---|
| **StratoVirt** | openEuler 安全容器主线后端 | 每个 OS、架构和 Kata 版本组合独立验证 |
| Cloud Hypervisor | 内容寻址 artifact、Profile 和 V2 runner 已接入的交叉验证后端 | 使用自身的 `configuration-clh.toml` 完成同一组接口和数据测试，不从 StratoVirt 结果外推 |

## OS 与架构适配矩阵

宿主 OS、guest OS、CPU 架构和 runtime/VMM 是四个独立维度。宿主 OS 通过不代表
同系列 guest 已支持,同一 VMM 在 x86_64 通过也不代表 aarch64 通过。
仓库归档的既有验证记录覆盖 openEuler x86_64 和 ARM64 的 host、guest rootfs
image、containerd/ctr、Kata 3.32 与 StratoVirt 组合。相应历史运行覆盖了二进制
兼容性、guest 服务生命周期、workload 访问与身份以及 TLS/eBPF 数据采集。签名镜像、
打包交付、升级回滚、Kubernetes RuntimeClass 和生产生命周期故障矩阵仍未完成。

| OS/guest 组合 | 已归档验证范围（非当前提交证据） | 仍需补齐 |
|---|---|---|
| Ubuntu | x86_64 Ubuntu 宿主 + Ubuntu guest 的 StratoVirt/Cloud Hypervisor 数据能力 e2e；候选 rootfs image 的 guest-root daemon/workload 接口双 VMM 验证 | aarch64、正式 guest image 与生命周期 |
| openEuler | x86_64 24.03 host/guest，以及 ARM64 24.09 host + 24.03 guest + 24.09 workload；两者均以 Kata 3.32.0、StratoVirt 2.4.0 完成接口和 TLS/eBPF 数据矩阵 | 签名镜像、打包交付、升级/回滚、真实 Collector、ARM64 Cloud Hypervisor 交叉验证与 Kubernetes 生命周期矩阵 |

每个环境组合都要分别检查 ABI/动态库、guest 内核 BTF/tracefs/BPF、服务启动、
workload 接口、eBPF/TLS 数据以及 shutdown/异常退出生命周期。系统差异只放在部署
脚本和配置中，不分叉共享采集核心。

ARM64 Kata 的声明仅限下方实测的 openEuler/Kata/StratoVirt 精确组合，不能由普通
容器证据或 x86_64 结果外推到其他 VMM、发行版和版本。

## 能力状态

| 能力 | 状态 |
|---|---|
| TLS 明文 + LLM 语义(tls-sync,免 eBPF) | V2 验收矩阵要求 OpenSSL `SSL_write`/`SSL_read` 双向明文命中；每个 OS/VMM 组合必须独立复验并附当前提交的运行记录 |
| eBPF 进程/文件/网络事件 | V2 验收矩阵要求带 BTF/tracefs 的 guest 内核产生非零事件；默认内核缺能力时两轴 `auto` 降级并显式标 degraded |
| guest-root workload 接口 | V2 验收矩阵要求 daemon 不进 workload，工具和 `/dev/actrail` socket 只读挂载，UID 1000/GID 39000 可用且错误 GID 明确拒绝 |
| workload 身份边界 | `SO_PEERCRED` 对端经 PID namespace + mount namespace 授权；`list-traces` 可查询根 PID namespace；container ID 不参与授权或验收 |
| 归属三键 `host.id` / `k8s.pod.uid` / `container.id` | 数据模型和 cgroup 解析代码就绪；guest-root 实测可见 containerd namespace/container ID。Kata 无 DMI 时不伪造 `host.id`，实时 OTLP trace ID 可回退到根 `container.id`；`host.id`/`k8s.pod.uid` 的 SQLite 持久化、真实 CRI/K8s 归属 e2e 与可选 runtime metadata 富化未完成 |
| 实时出境 `otel-http` | **候选能力**：已通过 builtin plugin 启动接口接入；JSON/protobuf、gzip、HTTP/1.1 keep-alive、HTTPS/mTLS、有界重试、idle batch timer、shutdown flush 和 queue/retry/drop/last-error 状态指标已有代码/协议测试；交付语义为 best-effort，WAL、真实 Collector 互操作、证书分发/轮换和 Kata 生命周期故障矩阵未完成 |
| 多容器 Pod(sidecar/init) | **当前不声明支持**：采集运行时已支持多 PID namespace 并发，但 guest 内共享 GID/control/TLS socket 尚不能提供逐 workload 授权隔离；MVP 限每个 sandbox 一个 workload 容器 |
| 独立 Kata VM 的 Agent 并发 | V2 验收矩阵要求两台 VM 各运行一个 xiaoO，双 Agent 存在明确重叠窗口；两边 trace 均 Clean 且 eBPF/网络事件非零，删除一台 VM 不影响另一台，测试结束无对应 shim/VMM 残留 |

## workload 授权、runtime 句柄与数据归属

guest-root daemon 不信任用户传入的 container ID。control socket 从
`SO_PEERCRED` 获取调用方 PID，再解析 PID namespace 与 mount namespace；两者共同
构成 workload 授权边界。attach 时捕获的 PID namespace symlink target（例如
`pid:[4026532248]`）保存在活动 trace 中，可由 `list-traces` 非阻塞查询。

V2 runner 仍需把 `CONTAINER_ID` 传给 `ctr run`、`kata-runtime exec` 和定向清理，
但它只是 containerd 生命周期句柄。测试不要求它具有 CRI/64-hex 形状，也不把它与
daemon 输出比较。

`host.id`、`k8s.pod.uid` 和 `container.id` 是三个生命周期不同的扁平 OTel
resource 属性，共同回答“哪一个运行边界产生了这条 trace”，不用于构建 agent 调用
DAG，也不参与上述 workload 授权：

| 属性 | 标识对象 | 变化时机 | 缺失后的影响 |
|---|---|---|---|
| `host.id` | daemon 所在 OS 实例：物理宿主、普通 VM 或 Kata microVM guest | 机器或 guest 重建 | 多机汇聚后无法可靠判断数据来自哪个采集边界 |
| `k8s.pod.uid` | Kubernetes Pod | Pod 重建；同一 Pod 内容器重启时通常不变 | 无法把主容器、sidecar、init container 归到同一 K8s 调度/策略对象 |
| `container.id` | 具体 runtime 容器 | 容器每次重建 | 无法区分同一 Pod 内多个容器和同一容器名的不同实例 |

这些字段按部署形态按需出现，不制造占位值。当前代码在 `track-add` 时用已解析的
root host PID 读取根 workload cgroup，得到 `container.id` 和可选
`k8s.pod.uid`；若 daemon 能取得 `host.id` 则一并写入，否则保持缺失。OTLP codec
只输出实际存在的 resource 属性。这个数据归属底座不等于完整 Kubernetes 支持：
真实 CRI/K8s e2e、runtime metadata 注入和多容器 Pod 逐容器归属仍未完成。

## 历史验证范围与当前分支要求

| 环境组合 | 历史验证范围 |
|---|---|
| Ubuntu x86_64 host + Ubuntu guest，StratoVirt / Cloud Hypervisor | Kata 启动、TLS/eBPF 单项与组合采集、guest-root 服务、非 root workload 接口、权限和身份 |
| openEuler 24.03 LTS-SP1 x86_64 host + openEuler 24.03 guest，containerd 2.3.3、Kata 3.32.0、StratoVirt 2.4.0 | 二进制和动态库兼容、rootfs 构建与注入、服务启停、workload 接口与身份、无 BTF 降级、BTF 内核 TLS/eBPF 采集 |
| openEuler 24.09 ARM64 host/cgroup v1 + openEuler 24.03 guest + openEuler 24.09 workload，containerd 1.6.22、Kata 3.32.0、StratoVirt 2.4.0 | preflight；verify/deny/launch/namespace；TLS-only、eBPF-only、combo；非 root GID 39000、root PID namespace 查询、双向 TLS 明文和非零 eBPF 事件；两台独立 Kata VM 各运行一个 xiaoO 的并发、trace 与生命周期隔离 |

V2 case 通过公共 Python Kata 生命周期管理器验证接口、TLS/eBPF 和双 VM xiaoO；
低层 Shell 仅保留部署契约与 guest fixture。完整转测入口和手动步骤见
[`../../tests/v2/regression/virtual_container/README.zh.md`](../../tests/v2/regression/virtual_container/README.zh.md)。
openEuler guest 的制作和复测步骤见
[`guest/OPENEULER.md`](guest/OPENEULER.md)，workload 接口命令见
[`workload/README.md`](workload/README.md)。

本表整理既有部署验证范围，不作为当前提交的完成证据，也不自动扩展到其他发行版、
版本或架构。当前分支只有在运行记录同时包含提交号、冻结版本、VM 数、耗时、第二次
缓存命中以及 task/container/shim/VMM 无泄漏结果时，才可重新声明“已验收”。

## 已知限制

- **kata-agent cgroup 布局已映射基础形态**:真实 guest fixture 为
  `0::/<containerd-namespace>/<container-id>`(cgroup v2 cgroupfs 风格)；等价的
  cgroup v1 多控制器格式
  `N:<controllers>:/<containerd-namespace>/<container-id>`已纳入回归 fixture。
  解析器共享 Kata 叶子 fallback，并保留 Docker、containerd/Kata 和 Kubernetes
  Pod UID 识别。openEuler ARM64 宿主 cgroup v1 已完成 StratoVirt E2E；该 Kata
  guest 通过内核参数使用 cgroup v2，因此“guest 内 cgroup v1 身份布局”仍只有
  fixture 证据。当前不声明 Podman 或 CRI-O 身份适配；未映射布局会 fail-loud，
  不静默弱化隔离。
- **microvm 无 DMI**:x86 Cloud Hypervisor 与 StratoVirt/Kata 已确认默认 microvm
  不模拟 SMBIOS，`host.id` 的 DMI 探测会落空。当前不使用 hostname 或目录名伪造
  `host.id`，而是保持缺失并让 trace ID 回退到根 `container.id`；若产品需要稳定的
  sandbox/node 维度，再通过 runtime metadata 增加语义独立的字段。
- **身份富化当前仅对实时出境生效**:`host.id` 和 `k8s.pod.uid` 当前不写入 SQLite。
  从 SQLite 重载的 trace 会丢失这两个属性；当实时 trace 曾以 `host.id` 派生 OTLP
  trace ID 时，重载后还可能改用 `container.id` 或本地 trace ID。当前候选能力只
  声明实时 OTLP 身份富化，不声明存储重放后的全局 trace ID 稳定性；schema 迁移和
  历史数据兼容留作后续。
- **Kubernetes 只消费本模块的接口契约**:`/dev/actrail` socket 与
  `/opt/actrail` 工具包的只读挂载、固定 GID 和非 root 权限已在双 VMM 验证。
  本模块负责稳定该接口及所需的身份、生命周期输入；后续 Kubernetes 集成层负责用
  RuntimeClass/CRI 实现挂载、GID、Pod UID 和 sandbox 退出通知，不在本模块内维护
  Kubernetes 部署资产。GID `39000` 当前适合单 workload MVP，不能无差别授予同
  sandbox 的所有不可信容器。
- **workload 私有动态库不进入 Agent 环境**:bundle 记录 `actrailctl` 的 ELF
  interpreter，并通过 loader `--library-path` 仅为客户端加载
  `/opt/actrail/lib`。launcher 不导出 `LD_LIBRARY_PATH`，避免 bundle 内 OpenSSL
  等依赖改变 Agent 的动态库选择。
- **新拓扑的本地数据检查已完成**：guest-root 测试已自动断言 BTF/eBPF
  启用、非零事件和 TLS payload 内容，namespace fixture 已比较 daemon 返回的 root
  PID namespace 与 workload 的 `/proc/self/ns/pid`。
  尚未完成的是 `container.id` 的 OTLP Collector 端断言、异常 sandbox shutdown
  flush 和 Cloud Hypervisor 上的 openEuler guest 交叉复验。
- **当前 Kata 组合不支持 AcTrail seccomp user-notify**:Kata 3.32 双 VMM
  实测自动启用会导致退出超时，因此 guest 配置关闭 `seccomp_notify` 和
  `process_seccomp`，同时关闭依赖 seccomp fallback 的 socket payload 轴和
  enforcement；workload launcher 显式选择
  `--seccomp-notify disabled`。这不表示 Kata/Linux 没有 seccomp；其他
  Kata/runtime 组合若要启用 AcTrail user-notify，必须单独完成版本兼容与退出
  生命周期验收。
- **rootfs image 与 initrd 不是同一条安装路径**:完整 rootfs image 的离线注入和
  systemd 启动已经验证；当前实现不适配 `kata-containers-initrd.img`。以后若出现
  明确客户需求，应作为独立部署方案重新设计和验收。
- 出境 v1 语义 = best-effort + 有界重试 + 响亮丢弃。HTTPS/mTLS 协议测试已经覆盖；
  at-least-once/WAL、真实 Collector 互操作、证书分发与轮换、异常 sandbox 退出可靠性
  属于整体部署阶段。

guest systemd 的 `optional` 不为 `actraild.service` 与
`kata-agent.service` 建立强制启动顺序，观测服务启动或停止失败不阻塞
kata-agent；`required` drop-in 才建立 `Requires/After` 依赖。这个选择不改变
通用 `actrailctl launch` 语义。当前
`workload/actrail-launch` 在配置、控制面或观测建立失败时仍可能阻止 Agent，因此
systemd fail-open 不等于 launcher fail-open。普通容器与 Kata 的共享 launcher
fail-open 尚未实现。

## 候选研究方向

Kata workload OCI seccomp 加固与 AcTrail eBPF/TLS-sync 观测可以作为独立轴组合，
并利用 pidfd 的 `clone3` 正常路径与 PR #90 的 `ENOSYS -> clone` 兼容路径，在不启用
AcTrail seccomp user-notify 的情况下同时验证 syscall 最小权限、观测完整性和身份
原子性。该方向仍是研究候选，不属于当前支持范围；问题定义、假设、验收矩阵和现有
证据见
[`../../docs/designs/virtual-container/hardened-observability-profile.zh.md`](../../docs/designs/virtual-container/hardened-observability-profile.zh.md)。

## 本目录资产

```
guest/operator.conf     guest 内 actraild 配置模板(通过 builtin plugin 启动接口加载
                        otel-http 实时出境；已固化两条易踩的坑:语义投影需显式开启、capture 需含
                        net-application-plaintext-http)
guest/actraild.service  guest-root systemd unit 模板(候选 rootfs image 已真实启动验收)
guest/systemd/required/...  required 启动依赖:kata-agent 强依赖 actraild ready。
guest/otel-endpoint.sh   校验 Guest 可达 endpoint 并原子渲染 otel-http 配置。
guest/install-rootfs.sh  向离线/已挂载 rootfs 安装最小 guest 服务,校验 checksum、
                        架构、GLIBC、空间和显式 Collector endpoint，并 enable unit。
guest/verify-rootfs.sh   离线检查安装布局、注入后的 endpoint 和启动依赖契约。
guest/Containerfile.openEuler  固定 openEuler guest image 和 24.09 Kata guest
                        kernel 的隔离构建依赖。
guest/build-openeuler-image.sh 使用 dnf installroot 和 mkfs.ext4 -d 构建无需
                        privileged 容器的 openEuler systemd 候选镜像。
guest/build-openeuler-kata-kernel.sh 从精确 openEuler 24.09 Kata SRPM 构建只增加
                        CONFIG_VIRTIO_FS 的 ARM64 候选内核并记录来源。
host/install-kata-3.32.sh 校验官方 ARM64 归档并将 Kata 3.32.0 并行安装到版本化
                        /opt 前缀，通过 /usr/local 激活且保留发行版 3.2 RPM。
host/prepare-stratovirt-config.py 从官方 Kata 3.32 配置生成指向候选 guest 内核、
                        镜像、virtiofsd 和所选 StratoVirt/Cloud Hypervisor 的配置；
                        文件名为兼容既有调用保留。
host/prepare-v2-test-artifacts.py 编排 bundle、双 guest image 注入、runtime config、
                        manifest 和 format 2 本机 profile，按输入摘要缓存并原子发布。
host/run-v2-tests.sh    读取本机 profile，一条命令运行两个虚拟容器 V2 cases。
guest/inject-image.sh    只复制并修改输出镜像,不改原始 Kata image；支持 ext4 直盘或
                        第一个分区,并贯通 workload socket GID 与 OTLP endpoint。
host-collector/          主机侧受限 Collector 验收部署；不是生产遥测后端。
guest/OPENEULER.md       openEuler Kata guest 的无特权构建主路径、可选副本注入、必要
                        agent-policy 资产、配置要求和复测步骤。
workload/prepare-bundle.sh  从已验证 guest bundle 生成不含 daemon/viewer 的最小
                        workload 工具包,重写路径、解析配置并原子发布。
workload/Containerfile.openEuler  构建 openEuler 24.09 workload rootfs，并为
                        containerd 1.6 的非 root fallback 提供 setpriv。
workload/actrail-init    由显式 `/bin/sh` 执行的 Kata workload PID 1 supervisor,
                        负责启动并回收 launch wrapper。
workload/actrail-launch  非 root launch wrapper:先验证只读 mount/GID/socket,再通过
                        guest-root daemon 启动 trace并保留有界收尾窗口。
workload/verify-interface.sh  校验 manifest、最小权限、socket 模式/GID、只读 mount、
                        doctor 和 TLS probe。
workload/README.md       guest `/dev` socket 源、固定 GID、安全边界和运行命令。
```

测试脚本、Kata runner 和 guest 测试夹具统一位于
[`../../tests/v2/regression/virtual_container/`](../../tests/v2/regression/virtual_container/)，
由 V2 runner 汇总，不作为部署资产打入 guest 或 workload。PID namespace 断言工具
也位于该目录，仅在 namespace 测试时额外挂载。runner 的 container ID 只作为
`ctr` 创建、exec 和定向清理句柄，不参与授权，也不作为测试通过判据。

**当前未包含的资产**:
`guest/kernel/`(等生产级带 BTF 内核和版本冻结);
`kata/configuration-*.toml`、
`runtimeclass.yaml`(由后续 Kubernetes 集成层按稳定接口契约提供);
`lifecycle.sh`(等完整 Kata 环境:shim-v2 + 足够内存的 KVM 机)。
