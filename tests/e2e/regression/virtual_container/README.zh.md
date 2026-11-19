# 虚机容器回归测试

## 测试目标

验证 Kata 虚机容器中的启动接口、权限和身份边界，以及 Guest 内 TLS 与 eBPF 观测的采集、落库和查询。

## 场景设计

每个 backend 顺序启动 base VM 和 data VM。base VM 覆盖 verify、deny、launch、namespace，data VM 覆盖 TLS、eBPF 和组合采集，分别核对实际工作负载及其观测结果。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储和查询观测记录 |
| 主机 | Linux、root、KVM、containerd、Kata 与所选 VMM | 创建和管理虚机容器 |
| 部署产物 | 与当前 checkout 匹配的 Guest 镜像、配置、manifest 和本机 profile | 启动 Guest 采集环境 |

## 测试步骤摘要

1. 检查宿主能力和当前 release 对应的部署产物。
2. 校验部署契约，并执行 runtime、VMM、配置和 cgroup 的 preflight。
3. 启动 base VM，依次执行接口、拒绝、启动和 namespace 场景。
4. 启动 data VM，完成 TLS 首次发现交互后执行三个数据场景。
5. 通过 viewer 核对 Guest 观测记录及归属。
6. 停止 VM 并检查本轮资源清理。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 接口与权限 | base VM 运行四个接口场景 | Guest root daemon 与非 root workload 的允许、拒绝和身份行为符合接口契约 |
| TLS 采集 | data VM 完成 TLS 工作负载 | TLS 明文被采集并可从所属 trace 查询 |
| eBPF 采集 | data VM 完成进程及网络工作负载 | 对应 eBPF 数据可落库和查询 |
| 组合采集 | TLS 与 eBPF 同时启用 | 两类数据均完整并关联到对应工作负载 |
| 部署条件 | 外部 KVM 或 backend 能力缺失 | 报告 SKIPPED 并说明缺少的条件 |

## 快速运行

公共 runner 默认使用 `VIRTUAL_CONTAINER_E2E_SCOPE=auto`。没有 `/dev/kvm` 的 Linux
开发机可直接运行下面的基础 case；contracts 通过后以明确的 `SKIPPED` 结束，不启动 VM：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

contracts 对 ELF 元数据的读取固定使用稳定的 C locale，因此宿主界面语言不会把有效的
release 误判为不可读；ELF、bundle 或静态契约本身损坏仍然属于 `FAILED`。只有
contracts 通过后确认 `/dev/kvm` 或 Kata backend 外部能力不可用时才是 `SKIPPED`。

检测到 `/dev/kvm` 时，完整 Kata runtime 验收需要先在同一 checkout 生成 profile 和 artifact，
并通过 `--profile` 显式加载。可显式设置 scope 为 `contracts` 或 `all` 覆盖自动选择。

首次部署必须先把目标分支拉到最终 checkout，并在该 checkout 中完成 release 构建和
`prepare-artifacts.py`。`local/kata/` 是 Git 忽略的 checkout-local 资产；
新 worktree 不会继承旧目录的 profile、manifest 或 bundle，也不应复制旧 profile 来
绕过准备。完整首次部署命令见
[`deploy/virtual-container/README.md`](../../../../deploy/virtual-container/README.md)。
preparer 应由部署用户通过 `sudo -E` 启动，而不是在 root shell 中直接运行；它完成需要
特权的准备后会把 checkout-local artifact 和 profile 的属主还原为该部署用户。

完成准备后，在同一个仓库根目录执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --profile local/kata/v2-test-profile.json \
  --case virtual_container \
  --color never
```

同时运行基础和双 xiaoO 并发 case 时省略 `--case`：

```bash
deploy/virtual-container/host/run-v2-tests.sh --profile local/kata/v2-test-profile.json --color never
```

脚本将显式指定的 profile 传给公共 runner，并在需要时申请一次 sudo。
指定的 profile 不存在时启动失败。密码、API Token 和私钥不会写入 profile。

## 手动测试

### 场景矩阵

单个 backend 使用两台顺序启动的长驻 Kata VM：

1. base VM 依次执行 `verify`、`deny`、`launch`、`namespace`；
2. data VM 先运行一次不判定的 TLS 首次发现交互（见 v2 文档“TLS 首次发现”），再依次执行 `tls-only`、`ebpf-only`、`combo`。

**采集配置**：Guest 配置由 `actrailctl init` 合并产品默认值与部署设置生成。三个数据场景均使用默认 eBPF 采集，名称表示验收内容。独立的 `guest/tls-only.sh` 覆盖显式 TLS sync 后端。

因此单 backend 共启动约 2 台 VM，最大同时运行 1 台。某个外部 backend/VMM 缺失
只会使该 backend 为 `SKIPPED`；部署产物过期、配置错误或数据断言失败为 `FAILED`。

### 详细说明

- [V2 测试、矩阵与排障](v2/README.zh.md)
- [Kata 部署与支持边界](../../../../deploy/virtual-container/README.md)
- [openEuler guest 制作](../../../../deploy/virtual-container/guest/OPENEULER.md)
- [workload 接入契约](../../../../deploy/virtual-container/workload/README.md)

普通 Docker 容器由 `container_auto` 和 `container_agent_xiaoo` 独立验证；其结果不能
替代 Kata guest 内独立内核、guest daemon 和 runtime Profile 的证据。

### 场景配置与运行环境

本功能验证 AcTrail 在 Kata 虚机容器中的两条关键链路：

- guest-root `actraild` 与非 root openEuler workload 的接口、权限和身份边界；
- guest 内 TLS 明文与 eBPF 数据的采集、落库和 viewer 查询。

当前公共 case 名称为 `virtual_container`，并由统一 V2 runner 管理。测试不会在运行时
安装 Kata、替换宿主内核或修改 containerd；这些属于显式部署准备步骤。

contracts scope 还会离线验证 Guest OTLP endpoint 必须显式注入、占位/loopback
地址会在写镜像前失败，以及主机侧 Collector 的版本固定、资源上限、只读根文件系统、
落盘轮转和非 WAL 边界。
