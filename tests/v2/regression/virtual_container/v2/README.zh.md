# Virtual Container V2

## Quick Run

完成一次与当前 release 一致的虚拟容器部署准备后，在仓库根目录执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

`run-v2-tests.sh` 是虚拟容器 V2 的宿主验收入口。它定位仓库和公共
`test_all.py`，读取机器本地的 `local/kata/v2-test-profile.json`；当前用户不是 root
时只申请一次 sudo，并保留调用用户的 Cargo、Rustup 和 PATH。脚本先执行
`release_install`，再校验内容寻址 artifact manifest，最后运行部署契约、Kata
preflight 和完整接口/数据矩阵。

该脚本不会安装或替换宿主 Kata、VMM、内核、containerd，也不会临时从其他
用户目录寻找配置。部署产物必须先由 artifact 准备器写入本机 profile。
修改 AcTrail release 代码后，必须按手动步骤2、3重新安装 release 并准备 artifact；
旧 manifest 会在启动 VM 前以 `checksum mismatch` 明确失败，脚本不会静默使用旧镜像。

单个所选 backend 的测试结构如下：

```text
openEuler host
  `-- containerd + io.containerd.kata332.v2
       +-- base Kata VM（顺序复用）
       |    `-- verify / deny / launch / namespace
       `-- data Kata VM（顺序复用）
            `-- tls-only / ebpf-only / combo
```

因此本 case 共启动 2 台顺序 VM，最大同时运行 1 台；data VM 至少使用 2 vCPU。
成功输出：

```text
▶ release_install
✓ release_install
▶ virtual_container...✓
```

## 步骤摘要

1. 检查 ARM64、KVM、containerd、Kata 3.32、shim、所选 VMM 和 source config。
2. 构建并安装当前 AcTrail release。
3. 生成内容寻址 guest image、bundle、runtime config、manifest 和本机 profile。
4. 校验 profile、manifest 与当前 release 的 SHA-256 一致。
5. 运行不访问 KVM 的部署契约和 Python 生命周期单元测试。
6. 运行 preflight，验证 runtime/VMM/config/cgroup 组合。
7. 启动一台 base VM，顺序执行 `verify/deny/launch/namespace`。
8. 启动一台 data VM，顺序执行 `tls-only/ebpf-only/combo`。
9. 检查日志和本轮资源清理结果。

## 手动测试

以下命令均从仓库根目录执行。多行命令的 `\` 必须位于待续行的末尾；不要把 `\`
单独写在下一行，也不要在 `\` 后追加空格。

先设置本机路径。将 `/path/to/...` 替换为当前机器上的实际绝对路径：

```bash
REPO_ROOT="$(pwd)"
BIN_DIR="$REPO_ROOT/target/release"
CTR_RUNTIME="io.containerd.kata332.v2"
BACKEND="${BACKEND:-stratovirt}"
BASE_CONFIG_SOURCE="/path/to/configuration-base-source.toml"
DATA_CONFIG_SOURCE="/path/to/configuration-data-source.toml"
XIAOO_BIN="/path/to/xiaoo"
```

默认测试使用 Guest 本地 SQLite 和镜像内的 `actrailviewer`，不需要 Collector。需要验证
OTLP/HTTP 外送时再设置：

```bash
GUEST_EGRESS_MODE="network"
GUEST_OTEL_ENDPOINT="http://<Guest 可达的主机 IP>:4318/v1/traces"
```

无 CNI、Guest 只有 loopback 且需要验证外送时，改为：

```bash
GUEST_EGRESS_MODE="vsock-bridge"
GUEST_OTEL_ENDPOINT="http://127.0.0.1:14318/v1/traces"
```

并先按
[`deploy/virtual-container/vsock-egress/README.md`](../../../../../deploy/virtual-container/vsock-egress/README.md)
安装、启用所选 backend 的 Host bridge。

### 步骤1：检查测试前提

#### 手动指令

```bash
test "$(uname -m)" = "aarch64"
test -c /dev/kvm
sudo test -r /dev/kvm
sudo test -w /dev/kvm
test -f "$BASE_CONFIG_SOURCE"
test -f "$DATA_CONFIG_SOURCE"
command -v ctr
command -v kata-runtime
command -v containerd-shim-kata332-v2
case "$BACKEND" in
  stratovirt) command -v stratovirt ;;
  cloud-hypervisor) command -v cloud-hypervisor ;;
  *) printf 'unsupported backend: %s\n' "$BACKEND" >&2; exit 2 ;;
esac
sudo ctr -n default version
```

#### 脚本行为与预期结果

runner 在启动 VM 前执行同类前提检查。`/dev/kvm`、containerd、shim 或 VMM 不可用
属于外部环境，相关 backend 标记为 `SKIPPED`；source config、manifest 或 release
损坏属于部署错误，case 标记为 `FAILED`。目标环境为 openEuler ARM64、Kata 3.32 和
所选 StratoVirt/Cloud Hypervisor backend；每个 backend 必须独立准备并验收。

### 步骤2：构建并安装当前 release

#### 手动指令

```bash
sudo -E env \
  "CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}" \
  "RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}" \
  "PATH=$HOME/.local/bin:$HOME/.cargo/bin:$PATH" \
  ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
  bash scripts/install-release.sh
```

#### 脚本行为与预期结果

`release_install` 构建并安装当前提交的 release 二进制。必须先完成本步骤，再生成
artifact；manifest 会记录 `actraild`、`actrailctl`、`actrailviewer` 和 TLS probe
的 SHA-256。release 后续发生变化时，旧 artifact 会在启动 VM 前以
`checksum mismatch` 明确失败。

### 步骤3：准备虚拟容器测试产物

#### 手动指令

同时准备基础和 xiaoO 并发测试：

```bash
sudo -E env \
  "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --backend "$BACKEND" \
    --base-config-source "$BASE_CONFIG_SOURCE" \
    --data-config-source "$DATA_CONFIG_SOURCE" \
    --xiaoo "$XIAOO_BIN"
```

只准备基础虚拟容器 case：

```bash
sudo -E env \
  "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --backend "$BACKEND" \
    --base-config-source "$BASE_CONFIG_SOURCE" \
    --data-config-source "$DATA_CONFIG_SOURCE"
```

containerd 中没有 workload image 时，可以使用离线 archive：

```bash
sudo -E env \
  "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --backend "$BACKEND" \
    --base-config-source "$BASE_CONFIG_SOURCE" \
    --data-config-source "$DATA_CONFIG_SOURCE" \
    --workload-image-archive /path/to/workload-image.tar \
    --image-pull-policy missing \
    --xiaoo "$XIAOO_BIN"
```

上面的三条准备命令默认不启用 exporter。若已设置 `GUEST_OTEL_ENDPOINT`，在所选命令中
追加：

```bash
    --otel-endpoint "$GUEST_OTEL_ENDPOINT" \
    --egress-mode "$GUEST_EGRESS_MODE"
```

#### 脚本行为与预期结果

准备器对 release、source image/config、kernel、VMM、virtiofsd、Guest 出境模式、部署
脚本和可选 xiaoO 计算内容摘要，在 staging 中构建完成后原子发布：

```text
local/kata/artifacts/<digest>/
├── manifest.json
├── guest-base.img
├── guest-data.img
├── configuration-base.toml
├── configuration-data.toml
├── guest-bundle/
├── host-bundle/              # actrail-vsock-gateway
├── workload-bundle/
└── xiaoo                    # 传入 --xiaoo 时存在
```

成功标志为 `ACTRAIL_V2_ARTIFACTS_READY`，并输出 `artifact_digest`、
`artifact_manifest` 和 `test_profile`。输入未变化时显示 `artifact_cache=hit`，不会
重新构建 bundle 或注入 guest image。

每次测试加载 manifest 时都会重新读取 base/data runtime config 的实际 `image`、
`kernel`、VMM `path` 和 `virtio_fs_daemon`，并与 manifest 输入摘要比较。外部 runtime
文件在准备后被原地替换、缺失或失去执行权限时，测试会在启动 VM 前判为 `FAILED`。

### 步骤4：检查 profile 和 artifact manifest

#### 手动指令

```bash
python3 -m json.tool local/kata/v2-test-profile.json

ARTIFACT_MANIFEST="$(python3 - <<'PY'
import json
from pathlib import Path

profile = json.loads(Path("local/kata/v2-test-profile.json").read_text())
print(profile["environment"]["VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"])
PY
)"
ARTIFACT_DIR="$(dirname "$ARTIFACT_MANIFEST")"

sudo python3 -m json.tool "$ARTIFACT_MANIFEST"
sudo test -f "$ARTIFACT_DIR/guest-base.img"
sudo test -f "$ARTIFACT_DIR/guest-data.img"
sudo test -f "$ARTIFACT_DIR/configuration-base.toml"
sudo test -f "$ARTIFACT_DIR/configuration-data.toml"
```

#### 脚本行为与预期结果

profile 只保存机器本地路径和运行参数，不保存密码、Token 或私钥。format 2 manifest
记录 base/data image、runtime config、bundle、workload image 和 release 摘要。
artifact 通常由 root 创建，普通用户不能读取时使用 sudo 只读检查；不要修改摘要目录
中的文件，否则后续校验应明确失败。

### 步骤5：运行不启动 VM 的契约测试

`VIRTUAL_CONTAINER_E2E_SCOPE` 默认是 `auto`：contracts 通过后，runner 检测
`/dev/kvm` 是否存在且当前用户可读写；不可用时自动停在 contracts 并返回明确的
`SKIPPED`，可用时继续完整 KVM/runtime 验收。协作者直接运行公共 runner 即可，
不需要先了解或设置该变量。使用 `contracts` 可强制只跑契约，使用 `all` 可强制进入
artifact/backend 验收并由缺失的 KVM 前提产生对应结果。

#### 手动指令

```bash
sudo -E env \
  "CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}" \
  "RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}" \
  "PATH=$HOME/.local/bin:$HOME/.cargo/bin:$PATH" \
  ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
  VIRTUAL_CONTAINER_E2E_SCOPE=contracts \
  python3 tests/v2/regression/test_all.py \
    --case virtual_container \
    --bin-dir "$BIN_DIR" \
    --color never
```

#### 脚本行为与预期结果

该显式模式运行部署 Shell 契约、公共 Kata 生命周期管理器单元测试、基础 case 模块测试和
并发 case 模块测试，不读取大型 guest image，也不访问 KVM。它适合无 KVM 开发机的
代码回归，但不能替代步骤7和步骤8的实机验收。contracts 全部通过后，顶层结果仍为
`SKIPPED`，避免把未运行 KVM/runtime 验收误报为完整通过：

```text
○ virtual_container — contracts passed; KVM runtime acceptance was not run
```

默认 `auto` 在无 KVM 主机上的结果会额外说明自动选择原因：

```text
○ virtual_container — auto-selected contracts because readable/writable /dev/kvm is unavailable; KVM runtime acceptance was not run
```

### 步骤6：运行宿主 preflight

#### 手动指令

```bash
sudo -E env \
  "PATH=$PATH" \
  BACKEND="$BACKEND" \
  CTR_RUNTIME="$CTR_RUNTIME" \
  RUNTIME_CONFIG_PATH="$ARTIFACT_DIR/configuration-base.toml" \
  tests/v2/regression/virtual_container/preflight.sh
```

#### 脚本行为与预期结果

preflight 校验 `/dev/kvm`、ARM64 KVM、containerd daemon、Kata runtime/shim 版本、
所选 VMM、runtime config 文件引用和宿主 cgroup v1 必要控制器。已验证环境应输出
`preflight: pass=11 fail=0`。该步骤只检查启动条件，不创建 VM。

### 步骤7：运行 base VM 接口矩阵

#### 手动指令

完整 case 会自动先运行 base VM。执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --no-cleanup \
  --color never
```

测试运行期间可在另一终端观察：

```bash
sudo ctr -n default tasks list | grep 'actrail-v2-' || true
sudo ctr -n default containers list | grep 'actrail-v2-' || true
ps -eo pid=,ppid=,args= | grep 'actrail-v2-' | grep -v grep || true
```

#### 脚本行为与预期结果

runner 创建一台长驻 base VM，并在同一 VM 中顺序执行四个 cell：

1. `verify`：UID 1000、补充 GID 39000 可以连接 guest control/TLS socket；
2. `deny`：错误 GID 在访问 AcTrail 前被拒绝；
3. `launch`：`actrail-init` 创建并完成 trace；
4. `namespace`：daemon 返回的 root PID namespace 与 workload 一致。

四个 cell 共用一台 VM，某个 cell 失败后不得以重新创建 VM 掩盖状态污染。

### 步骤8：运行 data VM 数据矩阵

#### 手动指令

步骤7的同一条命令会在 base VM 关闭后自动启动 data VM。查看阶段和数据标志：

```bash
grep -E \
  '(cloud|strato)\.(base|data|if\.(verify|deny|launch|namespace)|data\.(tls|ebpf|combo))' \
  /tmp/actrail-regression/logs/virtual_container.log
```

#### 脚本行为与预期结果

runner 使用开启 debug console、BTF/eBPF 和至少 2 vCPU 的 data Profile，顺序执行：

1. `tls-only`：`SSL_write/SSL_read` 双向明文 payload 完整；
2. `ebpf-only`：文件行为产生非零 eBPF event；
3. `combo`：同一 trace 同时包含 TLS、eBPF 和 network event。

每个 cell 都要求 guest-root control ready、openEuler workload、
`deployment_permissions_degraded=false` 和 `Completed/Clean` trace。全部完成后输出
`▶ virtual_container...✓`。

### 步骤9：检查日志和资源清理

#### 手动指令

```bash
tail -n 260 /tmp/actrail-regression/logs/virtual_container.log
find /tmp/actrail-regression/virtual_container \
  -maxdepth 3 \
  -type f \
  -print
sudo ctr -n default tasks list | grep 'actrail-v2-' || true
sudo ctr -n default containers list | grep 'actrail-v2-' || true
```

#### 脚本行为与预期结果

`--no-cleanup` 只保留 case 工作目录和日志，不保留 VM。生命周期管理器先验证随机
ownership label，再删除本轮 task/container，并等待删除前绑定的 shim/VMM 进程树
退出；任一资源残留都会使 case 失败。清理不得按宽泛前缀删除其他用户资源。cell
失败时日志中的 `Kata diagnostics` 会包含 task、container 和本轮宿主进程证据。

常见失败含义：

- `checksum mismatch`：release 变化后需要重新准备 artifact；
- `--xiaoo: command not found`：上一参数行末尾缺少 `\`；
- `pull policy is never`：先导入离线镜像，或准备时使用 `missing`；
- manifest `Permission denied`：使用 sudo 只读检查；
- data readiness 失败：检查 guest bundle 顶层是否为 `0755`；
- viewer JSON 失败：检查 debug console 的 Bash prompt/ANSI 控制码；
- regression lock 等待：已有另一轮 `test_all.py` 正在运行。

### 步骤10：运行全部虚拟容器 V2 用例

#### 手动指令

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --no-cleanup \
  --color never
```

#### 脚本行为与预期结果

省略 `--case` 时，包装脚本明确选择基础和双 xiaoO 并发两个 case：

```text
▶ virtual_container...✓
▶ virtual_container_xiaoo_concurrency...✓
```

普通 Docker 的 `container_auto` 不能替代该验收；Kata 还要求独立 guest 内核、
guest-root daemon、guest viewer、shim/VMM 生命周期和 `/dev/actrail` 接口证据。
