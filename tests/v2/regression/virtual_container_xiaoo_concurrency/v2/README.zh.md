# Virtual Container xiaoO Concurrency V2

## Quick Run

完成一次与当前 release 一致且包含 xiaoO 的 artifact 准备后，在仓库根目录执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container_xiaoo_concurrency \
  --color never
```

`run-v2-tests.sh` 是虚拟容器 V2 的宿主验收入口。它读取
`local/kata/v2-test-profile.json`，保留调用用户的 Cargo、Rustup 和 PATH，在需要时
申请一次 sudo，然后调用公共 `test_all.py`。选中本 case 后，runner 先执行
`release_install`，懒加载并校验 manifest 中的 data Profile、workload bundle 和
xiaoO，再启动双 VM 并发场景。
同一轮同时选择基础 `virtual_container` 时，若基础 case 为 `SKIPPED`，本 case
直接继承 `SKIPPED`，不再解析 artifact 或尝试启动 VM；单独选择本 case 时仍独立
检查全部前置条件。
修改 AcTrail release 代码后，必须重新安装 release 并准备 artifact；否则两个 case
都会在启动 VM 前以 `checksum mismatch` 失败，避免测试误用旧 guest 镜像。

并发场景不是在普通 Docker 容器中运行两个 xiaoO，而是同时创建两台独立 Kata data
VM。每台 VM 都有自己的 guest-root `actraild`、openEuler workload、本地 Provider
和 xiaoO：

```text
openEuler host
  +-- Kata data VM A
  |    `-- guest actraild -> workload A -> Provider A + xiaoO A
  `-- Kata data VM B
       `-- guest actraild -> workload B -> Provider B + xiaoO B

host barrier: provider.ready -> release -> xiaoo.active
```

Provider 是确定性的本地 OpenAI-compatible fixture，只监听各自 VM workload 的
loopback，不读取模型 Token，也不访问公网。测试固定同时启动 2 台 data VM，每台至少
2 vCPU，因此峰值至少需要 4 个 guest vCPU。

成功输出：

```text
▶ release_install
✓ release_install
▶ virtual_container_xiaoo_concurrency...✓
```

## 步骤摘要

1. 检查本机 profile、format 2 manifest、data Profile 和 xiaoO artifact。
2. 在需要时重新生成包含 xiaoO 的内容寻址 artifact。
3. 运行不启动 VM 的并发场景契约测试。
4. 创建两台独立 Kata data VM，并验证两边 workload 均为 openEuler。
5. 等待两个 Provider Ready，再统一释放 barrier。
6. 在同一轮询窗口看到两个 `xiaoo.active`，证明 Agent 真实重叠。
7. 验证 A/B response、文件 marker、trace 和 viewer 数据不会串线。
8. 删除 VM A 后确认 VM B 仍在运行，再清理本轮全部资源。

## 手动测试

以下命令均从仓库根目录执行。多行命令的 `\` 必须位于待续行的末尾；不要把 `\`
单独写在下一行，也不要在 `\` 后追加空格。

先设置公共变量：

```bash
REPO_ROOT="$(pwd)"
BIN_DIR="$REPO_ROOT/target/release"
PROFILE="$REPO_ROOT/local/kata/v2-test-profile.json"
BACKEND="${BACKEND:-stratovirt}"
BASE_CONFIG_SOURCE="/path/to/configuration-base-source.toml"
DATA_CONFIG_SOURCE="/path/to/configuration-data-source.toml"
XIAOO_BIN="/path/to/xiaoo"
```

将 `/path/to/...` 替换为当前机器上的实际绝对路径。

### 步骤1：检查测试前提和 xiaoO manifest

#### 手动指令

```bash
test "$(uname -m)" = "aarch64"
test -c /dev/kvm
sudo test -r /dev/kvm
sudo test -w /dev/kvm
test -f "$PROFILE"
python3 -m json.tool "$PROFILE"

ARTIFACT_MANIFEST="$(python3 - "$PROFILE" <<'PY'
import json
import sys
from pathlib import Path

profile = json.loads(Path(sys.argv[1]).read_text())
print(profile["environment"]["VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"])
PY
)"

sudo python3 - "$ARTIFACT_MANIFEST" <<'PY'
import json
import os
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
document = json.loads(manifest.read_text())
xiaoo = document.get("xiaoo")
if not xiaoo:
    raise SystemExit("manifest does not contain xiaoo")
path = Path(xiaoo["path"] if isinstance(xiaoo, dict) else xiaoo)
if not path.is_absolute():
    path = manifest.parent / path
print("manifest:", manifest)
print("xiaoo:", path)
if not path.is_file() or not os.access(path, os.X_OK):
    raise SystemExit(f"xiaoo is not executable: {path}")
PY
```

#### 脚本行为与预期结果

runner 只在选中本 case 时解析 xiaoO，因此 `test_all.py --list` 和基础
`virtual_container` 不依赖 xiaoO。manifest 中的 xiaoO 必须位于摘要目录、可执行且
SHA-256 正确；测试不从 `/home/<其他用户>/...` 自动寻找可变二进制。缺少 xiaoO 或
KVM/VMM 属于外部前提时标记为 `SKIPPED`，manifest 损坏则标记为 `FAILED`。

### 步骤2：准备包含 xiaoO 的 artifact

#### 手动指令

```bash
test -f "$BASE_CONFIG_SOURCE"
test -f "$DATA_CONFIG_SOURCE"
test -x "$XIAOO_BIN"

sudo -E env \
  "PATH=$PATH" \
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py \
    --backend "$BACKEND" \
    --base-config-source "$BASE_CONFIG_SOURCE" \
    --data-config-source "$DATA_CONFIG_SOURCE" \
    --xiaoo "$XIAOO_BIN"
```

#### 脚本行为与预期结果

准备器复制 xiaoO 到 `local/kata/artifacts/<digest>/xiaoo`，设为可执行并把相对路径及
SHA-256 写入 manifest，同时更新本机 profile。xiaoO 内容参与 digest；固定输入再次
执行显示 `artifact_cache=hit`，不会重复注入约 1 GiB 的 guest image。成功标志为
`ACTRAIL_V2_ARTIFACTS_READY`。

### 步骤3：运行不启动 VM 的并发契约测试

#### 手动指令

```bash
python3 -m unittest discover \
  -s tests/v2/regression/virtual_container_xiaoo_concurrency/v2 \
  -p 'test_*.py' \
  -q
```

#### 脚本行为与预期结果

该步骤检查 manifest 配置、data Profile、`KataContainerRequirements`、Provider
fixture、ownership label、双实例 marker 和生命周期契约，不访问 KVM，也不创建
task/container。单元测试通过只能证明编排契约正确，不能替代后续双 VM 实测。

### 步骤4：运行双 VM xiaoO 场景

#### 手动指令

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container_xiaoo_concurrency \
  --no-cleanup \
  --color never
```

需要直接调用公共 runner 时，等价命令为：

```bash
sudo -E env \
  "CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}" \
  "RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}" \
  "PATH=$HOME/.local/bin:$HOME/.cargo/bin:$PATH" \
  ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
  python3 tests/v2/regression/test_all.py \
    --case virtual_container_xiaoo_concurrency \
    --bin-dir "$BIN_DIR" \
    --no-cleanup \
    --color never
```

#### 脚本行为与预期结果

runner 校验 data Profile 必须开启 debug console、至少 2 vCPU，guest kernel 必须具备
BTF/eBPF。随后复制确定性的 xiaoO/Provider/workload 资产，为 A/B 创建不同的随机
container ID、run label 和协调目录，并同时启动两台 VM。两边必须保持 Running，
workload OS 必须为 openEuler，control socket、Python、Provider 和 xiaoO 均可用。

### 步骤5：确认两台 VM 和两个 Agent 真实重叠

#### 手动指令

主终端运行步骤4时，在另一终端执行：

```bash
sudo ctr -n default tasks list | grep 'actrail-v2-xiaoo-' || true
sudo ctr -n default containers list | grep 'actrail-v2-xiaoo-' || true
```

定位本轮协调目录并观察 barrier：

```bash
RUN_DIR="$(find /tmp/actrail-regression/virtual_container_xiaoo_concurrency \
  -maxdepth 1 \
  -type d \
  -name 'run-*' \
  -printf '%T@ %p\n' | \
  sort -nr | \
  sed -n '1s/^[^ ]* //p')"

printf 'run_dir=%s\n' "$RUN_DIR"
find "$RUN_DIR/coord" -maxdepth 2 -type f -print
```

#### 脚本行为与预期结果

应同时看到两个属于本轮的 Running task 和对应 container。A/B workload 各自启动
本地 Provider，写入 `provider.ready`；只有两边都 Ready 后，宿主才同时写入
`release`。runner 会按本轮 task 绑定并验证对应的 shim/VMM 进程，不使用宿主全局进程
数量作为通过条件。runner 必须在同一轮询窗口读到两个 `xiaoo.active` 才通过
`agent_overlap`。xiaoO 退出时会删除 `xiaoo.active`，因此最终目录中没有该文件是正常
清理行为，不表示未发生重叠。

### 步骤6：验证 A/B 输出和文件 marker 不串线

#### 手动指令

```bash
sed -n '1,260p' "$RUN_DIR/workload-a.log"
sed -n '1,260p' "$RUN_DIR/workload-b.log"
grep -F 'ACTRAIL_KATA_XIAOO_A_FILE_WRITE_OK' \
  "$RUN_DIR/coord/a/task-output.txt"
grep -F 'ACTRAIL_KATA_XIAOO_B_FILE_WRITE_OK' \
  "$RUN_DIR/coord/b/task-output.txt"
! grep -F 'ACTRAIL_KATA_XIAOO_B_' "$RUN_DIR/workload-a.log"
! grep -F 'ACTRAIL_KATA_XIAOO_A_' "$RUN_DIR/workload-b.log"
```

#### 脚本行为与预期结果

A 日志必须包含 `ACTRAIL_KATA_XIAOO_A_RESPONSE_OK`，B 日志必须包含
`ACTRAIL_KATA_XIAOO_B_RESPONSE_OK`；各自 task output 只能包含自己的文件 marker。
两边都必须输出 `host_ebpf:enabled,seccomp_notify:disabled`、
`deployment_permissions_degraded=false` 和 `KATA_XIAOO_WORKLOAD_OK`。任一 A/B marker
出现在另一边都判为 cross-trace isolation 失败。

### 步骤7：验证 trace 和 VM 生命周期隔离

#### 手动指令

```bash
grep -E \
  '(vms|providers|overlap|traces)|virtual_container_xiaoo_concurrency.*[✓✗]' \
  /tmp/actrail-regression/logs/virtual_container_xiaoo_concurrency.log

sudo ctr -n default tasks list | grep 'actrail-v2-xiaoo-' || true
sudo ctr -n default containers list | grep 'actrail-v2-xiaoo-' || true
```

#### 脚本行为与预期结果

在 VM 存活期间，runner 分别进入两边 guest root，按随机唯一 title 查询 viewer JSON。
每条 trace 必须为 `Completed/Clean`，且 `events > 0`、`network_events > 0`。这些证据在
VM 删除前由 runner 读取并断言，不依赖宿主共享数据库。

随后 runner 先删除 VM A，并立即确认 VM B 仍为 Running；最后删除 VM B。每次删除都
会验证本轮 ownership label，并等待对应 task/container/shim/VMM 全部消失。删除 A
导致 B 消失或任一本轮宿主进程残留都属于生命周期隔离失败，不允许通过重试掩盖；
失败日志会输出两台 VM 各自的 `Kata diagnostics`。

### 步骤8：检查日志、缓存和完整套件

#### 手动指令

```bash
tail -n 300 \
  /tmp/actrail-regression/logs/virtual_container_xiaoo_concurrency.log
find "$RUN_DIR" -maxdepth 3 -type f -print

deploy/virtual-container/host/run-v2-tests.sh \
  --no-cleanup \
  --color never
```

#### 脚本行为与预期结果

`--no-cleanup` 只保留 `run-<id>` 目录和主日志，不保留 VM。再次使用相同 manifest
运行不会复制或注入 artifact；Provider 始终本地运行，不需要 Token 或网络。

完整套件预期输出：

```text
▶ virtual_container...✓
▶ virtual_container_xiaoo_concurrency...✓
```

常见失败含义：

- manifest 中没有 xiaoO：重新执行步骤2；
- `--xiaoo: command not found`：上一参数行末尾缺少 `\`；
- Provider 未 Ready：检查 `coord/<instance>/provider.stderr`；
- 未同时出现两个 `xiaoo.active`：检查 barrier 文件和 A/B workload 日志；
- xiaoO 返回码非零：检查 `coord/<instance>/xiaoo.stdout`；
- trace 不存在：检查 workload 是否通过 `actrail-init` 启动及 control socket；
- `events=0`：检查 data kernel 的 BTF/eBPF、tracefs 和 bpffs；
- regression lock 等待：已有另一轮 `test_all.py` 正在运行。

普通 `container_agent_xiaoo` 使用 Docker 容器和宿主内核，不能替代本用例的两套
guest daemon、独立 guest 内核、guest viewer、shim/VMM 和 VM 生命周期隔离证据。
