# StratoVirt 执行隔离真实 xiaoO 测例

## 快速运行

完成下文的宿主前置条件并构建当前 checkout 的 release 后，从仓库根目录
先生成 StratoVirt 本机 profile：

```bash
ACTRAIL_HOST_PATH="$HOME/.cargo/bin:$HOME/.local/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/bin:/usr/bin"
ACTRAIL_HOST_PATH="$ACTRAIL_HOST_PATH:/usr/local/sbin:/usr/sbin:/sbin:/bin"

sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 \
  deploy/virtual-container/host/prepare-v2-test-artifacts.py \
  --backend stratovirt \
  --data-kernel /absolute/path/to/bootable-debug-kernel \
  --with-sandbox-observer \
  --xiaoo /absolute/path/to/xiaoo \
  --write-profile "$PWD/local/kata/v2-test-profile-stratovirt.json"
```

只有准备器打印 `ACTRAIL_V2_ARTIFACTS_READY` 后才能运行 case。准备失败时 profile
不会更新；继续使用旧 profile 只会产生 release checksum mismatch。

再运行告警用例：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --profile local/kata/v2-test-profile-stratovirt.json \
  --case execution_isolation_stratovirt \
  --color never
```

`run-v2-tests.sh` 会在需要时申请 sudo，并保留调用用户的 `CARGO_HOME`、
`RUSTUP_HOME`、`~/.cargo/bin`、`~/.local/bin` 和现有 `PATH`。它不会生成
profile。

`local/kata/` 是本机且被 Git 忽略的目录。profile 和 artifact 绑定生成它们的
机器与 checkout，不得从其他机器或其他 checkout 复制使用。

该可选测例启动一台由 Kata/containerd 管理的真实 StratoVirt VM，在 Guest 内运行
Guest systemd 注入的 root `actrail-sb` 和 artifact manifest 固定的真实 xiaoO。
case 先确认 observer ready 且尚未连接，再启动 Host gateway 并显式执行 Guest
`actrail-sb connect`，最后才启动非特权 workload；workload 不会另起第二个 SB。
StratoVirt 使用
内核 AF_VSOCK，因此 Host gateway 以 `native` backend 在 port `43182` 监听，不使用
Cloud Hypervisor 或 Firecracker 的 Unix socket 规则。

测例要求 xiaoO 亲自完成一次文件读取和一次文件写入，随后验证以下五类告警同时写入
独立 sandbox alert SQLite，并由公开告警订阅连接收到相同内容：

- `sandbox.resource.high_cpu`
- `sandbox.resource.oom_killed`
- `sandbox.resource.oom_risk`
- `sandbox.process.high_read`
- `sandbox.process.high_write`

## 准备

先在目标 Linux/KVM 主机上从同一 checkout 刷新 StratoVirt 产物；`--xiaoo` 必须指向
要演示的真实 xiaoO。快速运行第一步的 `prepare-v2-test-artifacts.py` 命令会生成
内容寻址 artifact 和本机 profile。

## 直接运行公共 `test_all.py`（仅调试）

如需绕过推荐 wrapper，复用本页快速运行开头定义的 `ACTRAIL_HOST_PATH`，
并显式将它传给 sudo：

```bash
sudo -E /usr/bin/env \
  "PATH=$ACTRAIL_HOST_PATH" \
  /usr/bin/python3.11 tests/v2/regression/test_all.py \
  --profile local/kata/v2-test-profile-stratovirt.json \
  --case execution_isolation_stratovirt
```

缺少 KVM、StratoVirt 或 Kata/containerd 属于外部条件，结果为 `SKIPPED`；release、
manifest、xiaoO 身份不一致属于部署错误，结果为 `FAILED`。

xgovernor 不在本测例内启动或修改。测例中的 subscriber 使用与 xgovernor 相同的
公开告警出口，证明它后续接入时所依赖的数据链已经成立。
