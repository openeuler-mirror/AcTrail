# Kata VMM 共享执行隔离告警场景

## 快速运行

完成下文的宿主前置条件并构建当前 checkout 的 release 后，从仓库根目录
先生成 Cloud Hypervisor 本机 profile：

```bash
ACTRAIL_HOST_PATH="$HOME/.cargo/bin:$HOME/.local/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/bin:/usr/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/sbin:/usr/sbin:/sbin:/bin"

sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 \
  deploy/virtual-container/host/prepare-v2-test-artifacts.py \
  --backend cloud-hypervisor \
  --data-kernel /absolute/path/to/bootable-debug-kernel \
  --with-sandbox-observer \
  --xiaoo /absolute/path/to/xiaoo \
  --write-profile "$PWD/local/kata/v2-test-profile-ch.json"
```

只有准备器打印 `ACTRAIL_V2_ARTIFACTS_READY` 后才能运行 case。准备失败时 profile
不会更新；继续使用旧 profile 只会产生 release checksum mismatch。

再运行告警用例：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --profile local/kata/v2-test-profile-ch.json \
  --case execution_isolation_cloud_hypervisor \
  --color never
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 `CARGO_HOME`、
`RUSTUP_HOME`、`~/.cargo/bin`、`~/.local/bin` 和现有 `PATH`。它只负责
环境传递和测试运行，不会生成 profile。

`local/kata/` 是本机且被 Git 忽略的目录。profile 和 artifact 绑定生成它们的
机器与 checkout，不得从其他机器或其他 checkout 复制使用。

Cloud Hypervisor、StratoVirt 和 Firecracker 共用这一套 alert runtime。三者都由
containerd/Kata 创建 VM，并使用 Guest systemd 启动的 root `actrail-sb` 采集；workload
只运行非特权真实 xiaoO，不再启动第二个 sandbox observer。

采集链统一为：

```text
Guest system actrail-sb → VSOCK → Host actrail-vsock-gateway → actraild
```

三种 VMM 仅在 VSOCK 与文件传输适配层不同：

| backend | Host VSOCK 发现与 gateway 参数 | workload 资产与协调 |
| --- | --- | --- |
| Cloud Hypervisor | 发现 `/run/vc/vm/*/clh.sock`，gateway 监听 `<base>_43182` | Kata 共享目录 |
| StratoVirt | native AF_VSOCK port `43182` | Kata 共享目录 |
| Firecracker | 发现 `/run/vc/firecracker/*/root/kata.hvsock`，把 base UDS 和 port 交给 gateway，由 gateway 监听 `<base>_43182` | artifact image 预装固定哈希 xiaoO；VM 启动后经 `ctr tasks exec` stdin 只制备小型资产，协调文件也经 Guest exec 访问，container spec 无 Host mount |

## Guest observer 契约

artifact profile 对三个 backend 都要求 `sandbox_observer_enabled=true`。Guest image 内：

- `actrail-sb.service` 默认启用，daemon control socket 为
  `/dev/actrail/sandbox-observer-control.sock`；
- `actrail-sb-connect.service` 仍安装并保留 `WantedBy`，但默认不启用，避免在 case-owned
  Host gateway 建好前自动 connect；生产环境需要自动连接时可显式 enable 该 unit；
- readiness marker 与诊断日志分别位于
  `/dev/actrail/sandbox-observer.ready` 和
  `/dev/actrail/sandbox-observer.log`。

每轮 case 的顺序固定为：

1. Kata VM 启动，GuestConsole 有界等待 system observer control socket、ready marker、
   `connected=false publication_enabled=false` 启动证据，并确认 auto-connect unit 未启用且
   未运行；
2. Host gateway 启动；Cloud Hypervisor/Firecracker 还必须等待实际
   `<base>_43182` UNIX listener 成为 socket；
3. case 通过 GuestConsole 在 VM root 执行当前 Guest 安装的
   `/usr/local/bin/actrail-sb connect`，握手成功后才进入 workload；
4. Firecracker 制备 Guest-local 资产，随后以 workload UID/GID 启动真实 xiaoO。

若显式 connect 或 listener gate 失败，case 会立即终止已启动的 gateway；统一 cleanup
只管理 workload、gateway、Kata VM、daemon 与 alert proxy，不存在第二个 workload SB
进程。

## 准备与运行

在目标 Linux/KVM 主机从同一 checkout 构建 release，并按目标 backend 刷新内容寻址
artifact。真实 xiaoO 和可引导、带 BTF/eBPF 的 data kernel 必须显式提供；快速运行第一步
的 `prepare-v2-test-artifacts.py` 命令会生成内容寻址 artifact 和本机 profile。

StratoVirt 和 Firecracker 应分别使用各自 README 开头的快速运行命令；切换 backend 时
必须同时把 `--write-profile` 改为对应 backend 的独立 profile，不得覆盖或复用
Cloud Hypervisor profile。Firecracker 还必须追加
`--workload-image-archive /absolute/path/to/workload.docker.tar`，供准备器生成预装固定哈希
xiaoO 的派生 image；该文件须满足 Firecracker case README 中的 `ctr images export`
combined archive 契约。每个 case 使用各自 profile。Firecracker 还要求 `dmsetup` 和状态为
`ok` 的 containerd `io.containerd.snapshotter.v1/devmapper` plugin；前置检查通过
`ctr plugins list` 的精确 type/id filter 验证，不解析 containerd 配置文本。

### 直接运行公共 `test_all.py`（仅调试）

如需绕过推荐 wrapper，复用本页快速运行开头定义的 `ACTRAIL_HOST_PATH`，
并显式将它传给 sudo：

```bash
sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 tests/v2/regression/test_all.py \
  --profile local/kata/v2-test-profile-ch.json \
  --case execution_isolation_cloud_hypervisor
```

共享场景要求真实 xiaoO 完成一次文件读和一次文件写，并验证五类告警同时写入独立
sandbox alert SQLite，且由公开 subscriber 收到相同 source/extras：

- `sandbox.resource.high_cpu`
- `sandbox.resource.oom_killed`
- `sandbox.resource.oom_risk`
- `sandbox.process.high_read`
- `sandbox.process.high_write`

release、manifest、runtime config、Guest observer 或 xiaoO 摘要不一致返回 `FAILED`；
KVM、containerd、Kata、shim、VMM 或 Firecracker devmapper 外部能力缺失返回
`SKIPPED`。
