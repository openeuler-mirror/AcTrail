# Kata workload 接入契约

本目录解决的是 **guest 根中的 `actraild` 如何被非 root workload 使用**。daemon
不进入 workload 镜像；workload 只获得客户端、TLS probe 和两个 Unix socket。

> 当前 `actrail-launch` 使用共享的通用 launch 语义，观测建立失败仍可能阻止
> Agent。当前实现不提供 Kata 专用的 fail-open；普通容器与 Kata 的共享 launcher
> fail-open、seccomp 运行期接管和显式 Required observation 均尚未实现。

## 边界与路径

```text
Kata guest root
├── actraild (root:actrail)
├── /run/actrail/private/       0700，仅 daemon 可见
│   ├── actrail.sqlite
│   ├── actraild.log
│   └── export/
└── /dev/actrail/               0750，guest 侧稳定挂载源
    ├── control.sock            0660 root:actrail
    └── tls-sync.sock           0660 root:actrail

workload mount namespace (UID 1000，GID 39000)
├── /opt/actrail/               只读：actrailctl、launcher、probe、依赖和配置
│   ├── bin/actrail-init        workload PID 1，回收 launch wrapper
│   └── bin/actrailctl-private  仅为 actrailctl 应用 bundle 私有动态库搜索路径
└── /run/actrail/               只读挂载 guest /dev/actrail
    ├── control.sock
    └── tls-sync.sock
```

普通 guest `/run` 不会自动出现在 workload mount namespace。Kata 对源路径位于
`/dev` 下的设备 bind mount 有 guest 侧处理，因此本实现使用 `/dev/actrail` 作为
稳定源，再映射到 workload 的 `/run/actrail`。这是显式 runtime 集成，不是 daemon
启动后任意 workload 都能自动看到 socket。依据：
[Kata Containers Limitations - device handling](https://github.com/kata-containers/kata-containers/blob/main/docs/Limitations.md)。

ARM64 主线固定使用 Kata 3.32.0。openEuler 24.09 RPM 中的 Kata 3.2.0 对该挂载的
路径分类逻辑过旧，会在宿主解析 guest-only 的 `/dev/actrail` 并报
`Could not resolve symlink for source /dev/actrail`。本项目不再为 3.2 创建宿主
`/dev` 标记目录；host runtime、guest agent 和配置必须一起对齐到 3.32.0。安装器
额外提供 `io.containerd.kata332.v2`（对应
`/usr/local/bin/containerd-shim-kata332-v2`），避免 containerd 优先命中发行版的
`/usr/bin/containerd-shim-kata-v2`。

固定数值 GID `39000` 是当前单 workload MVP 的授权边界：

- guest rootfs 安装时创建 `actrail:x:39000:`；名称或数值冲突会直接中止；
- workload 必须以主 GID 或 supplemental GID `39000` 运行；
- 工具和 socket 目录均只读挂载，workload 不能替换客户端、删除 socket 或读取
  daemon 的 SQLite、日志和导出文件；
- 拥有该 GID 的进程可以调用 control API，也可以发送 TLS 事件。多租户 sandbox
  不能把这个 GID 无差别授予所有容器，后续需按容器拆 socket/凭据或增加鉴权代理。

采集运行时已支持多 PID namespace 并发；当前每个 Kata sandbox 只声明支持一个
workload 容器，是因为共享 GID 和 socket 尚不能实现逐 workload 授权隔离，不是
eBPF 采集路由限制。

## 构建

ARM64 openEuler 主线先构建带 `setpriv` 的 openEuler 24.09 基础 workload 镜像。
这既保证容器内部 `/etc/os-release` 是 openEuler，也兼容没有 `ctr run --user`
的 openEuler containerd 1.6：

```bash
docker build \
  --build-arg BASE_IMAGE=openeuler-24.09:latest \
  -f deploy/virtual-container/workload/Containerfile.openEuler \
  -t actrail-openeuler-workload:24.09 \
  deploy/virtual-container/workload
```

然后生成并验证 guest bundle，再生成不含 daemon/viewer 的 workload bundle：

```bash
./tests/v2/regression/virtual_container/prepare-guest-bundle.sh
./deploy/virtual-container/workload/prepare-bundle.sh
./tests/v2/regression/virtual_container/test-prepare-workload-bundle.sh
```

`prepare-bundle.sh` 会校验 guest manifest、重写 workload socket/probe 路径、用真实
`actrailctl` 解析生成的配置，再原子替换输出目录。默认输出为
`.actrail-workload-bundle/`。

`WORKLOAD-INTERFACE` 记录 `actrailctl` 的 ELF `program_interpreter`。
`actrailctl-private` 先用 loader 的 `--verify` 检查 ELF/架构，再用
`--library-path /opt/actrail/lib --list` 验证运行时依赖，最后才执行客户端。ABI、
interpreter 或 glibc/共享库不兼容会在 Agent 启动前给出明确诊断。本实现不捆绑私有
基础 glibc/loader，也不承诺任意 workload 发行版兼容。

私有 library path 只应用于 `actrailctl`，不会导出或覆盖 `LD_LIBRARY_PATH`。因此
Agent 保留调用前的动态库环境，不会优先加载 AcTrail bundle 中的 OpenSSL 等依赖。

## Kata 验证

先按[虚拟容器部署说明](../README.md#测试资产的一键准备与运行)生成内容寻址
artifact 和本机 profile，再通过唯一的公共 V2 入口运行接口矩阵：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

V2 runner 默认使用 `docker.io/library/actrail-openeuler-workload:24.09`，并断言
`/etc/os-release` 中 `ID=openEuler`。其他发行版属于独立 Profile，必须重新生成
artifact manifest，不能通过关闭 OS 门禁复用 openEuler 验收结论。

接口矩阵的四个 cell 分别证明：

| 模式 | 通过条件 |
|---|---|
| `verify` | 非 root workload 可读两条只读挂载、连接 daemon、发现 TLS probe |
| `deny` | 错误 GID 在访问 socket 前被拒绝，并返回预期权限诊断 |
| `launch` | workload 通过 guest-root daemon 创建并完成 trace |
| `namespace` | daemon 返回的 root PID namespace 与 workload 的 `/proc/self/ns/pid` 一致 |

`namespace` 在 trace 建立后读取 workload 自身的 `/proc/self/ns/pid`，再通过只在
V2 namespace 模式挂载的测试工具调用 `list-traces`，与 daemon 返回的 root PID
namespace 比较。该断言工具不进入正式 workload bundle。公共生命周期管理器为每轮
生成唯一 container ID，并用 ownership label 约束定向清理；该 ID 不参与授权，也不
作为测试通过判据。

当前 Kata Profile 不支持 AcTrail seccomp user-notify：guest 配置关闭
`seccomp_notify`、`process_seccomp`、依赖 seccomp fallback 的 socket payload 轴和
enforcement，`actrail-launch` 同时显式使用
`--seccomp-notify disabled`。在已验证的 Kata 3.32 Cloud
Hypervisor/StratoVirt 组合中，自动启用该路径会在 workload 退出阶段触发 kata-agent
超时；TLS sync 与 guest eBPF 不依赖该能力。若以后启用 seccomp 路径，必须作为单独
runtime profile 重新验收，不能删除这个显式设置后假定
兼容。

runtime 必须以
`/bin/sh /opt/actrail/bin/actrail-init <launch-args>` 作为 workload 入口，而不是把
任一 shebang 脚本或 `actrailctl` 直接作为 OCI init。实机中省略显式 shell，或跳过
`actrail-init`，都会在受监督子进程退出后触发 Kata sandbox-stop 竞态；
`actrail-init` 提供独立 PID 1 回收层。这个要求已经写入 V2 场景，不能只靠镜像
默认 `ENTRYPOINT` 猜测。

launcher 在 `actrailctl` 完成 `track-remove` 后默认保留 2 秒
`ACTRAIL_WORKLOAD_EXIT_GRACE_SECONDS` 收尾窗口。实机中直接让 workload PID 1 退出会
使 shim 在 sandbox stop 时遇到 kata-agent `CheckRequest timed out`；2 秒窗口在两个
VMM 上均可正常返回。该值只接受 `0..60` 的整数，生产 profile 不应设为 `0`，除非
对应 Kata 版本已经单独通过立即退出测试。它是当前 Kata 3.32 Profile 的兼容措施，
不属于稳定 Guest/workload interface contract；新的 runtime/Profile 必须重新测量，
长期应由明确的 sandbox flush/退出握手替代。它不是 exporter 的持久化保证；完整
交付仍要验证 daemon 的 shutdown flush。

这套 V2 矩阵是 `ctr` 级接口证据。后续 Kubernetes 集成层只需遵守本目录定义的
接口契约：用 RuntimeClass/CRI 配置两条只读 mount 和 GID，并向 AcTrail 提供
Pod UID 与 sandbox 退出通知。本模块不维护 Kubernetes 部署资产。

矩阵、证据、手动复现和日志说明集中在
[V2 测试文档](../../../tests/v2/regression/virtual_container/v2/README.zh.md)，本页只维护
workload 接口与打包约束。
