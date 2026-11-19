# 虚机容器 xiaoO 并发回归测试

## 测试目标

验证两台 Kata VM 中真实 xiaoO 的并发执行、观测隔离和 VM 生命周期隔离。

## 场景设计

两台独立 data VM 各自运行 Guest daemon、本地 Provider 和 xiaoO。等待两个 Provider 就绪后统一释放 barrier，建立真实重叠窗口，分别保存 A/B 的实际回复，以回复正文、实际文件内容和 trace 身份验证隔离，再删除 VM A 检查 VM B。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储和查询观测记录 |
| 主机 | Linux、root、KVM、containerd、Kata 与所选 VMM | 创建和管理虚机容器 |
| 部署产物 | 与当前 checkout 匹配的 Guest 镜像、配置、manifest 和本机 profile | 启动 Guest 采集环境 |
| xiaoO | artifact 包含真实 xiaoO | 运行两个独立任务 |
| 资源 | 同时运行两台 VM，每台至少 2 vCPU | 建立并发窗口 |

## 测试步骤摘要

1. 检查宿主能力、profile 和包含 xiaoO 的 artifact。
2. 启动两台 data VM，确认 workload 环境和各自 Provider 就绪。
3. 统一释放 barrier，确认两个 xiaoO 同时活动。
4. 完成任务，保存 A/B 的完整实际回复和文件内容，再按各自 trace 身份核对观测。
5. 删除 VM A，确认 VM B 继续运行，再清理本轮资源。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 真实并发 | 两个 Provider 就绪后释放 barrier | 两台 VM 同时 Ready，两个 xiaoO 在同一窗口 Active |
| 观测隔离 | 已保存两个 Agent 的实际回复与文件操作结果 | 各自 trace 的响应包含对应 Agent 的完整实际回复，文件观测匹配实际路径和内容 |
| trace 完整性 | 采集完成 | 两条 trace 均 Completed/Clean，包含 eBPF/network 证据 |
| 生命周期隔离 | 删除 VM A | VM B 继续运行 |
| 可选 OpenCode | 启用免费模型 smoke，分别取得两个 Agent 的实际回复 | 对应 trace 的采集响应包含各自完整实际回复，归属与 workload 身份一致 |

## 快速运行

部署 manifest 中已经包含 xiaoO 后执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --profile local/kata/v2-test-profile.json \
  --case virtual_container_xiaoo_concurrency \
  --color never
```

两个虚机容器 case 一键运行：

```bash
deploy/virtual-container/host/run-v2-tests.sh --profile local/kata/v2-test-profile.json --color never
```

同一轮中基础 `virtual_container` 为 `SKIPPED` 时，并发 case 同样为 `SKIPPED`；
单独选择并发 case 时仍独立检查 KVM、artifact 和 xiaoO。

## 手动测试

### 完整验收条件

**采集配置**：两台 Guest 均使用 `actrailctl init` 合并产品默认值与 Guest 部署设置生成的配置，通过 eBPF 采集 Agent 行为。

- 两台 Kata VM 同时处于 Ready；
- 两个 Provider Ready 后统一释放 barrier；
- 两个 xiaoO 进程存在同一活跃窗口；
- 启用 OpenCode smoke 时，分别保存两边实际回复，并在各自 trace 中核对完整正文。
- A/B 的实际回复、文件操作与 trace 归属一致。
- 两条 trace 均为 `Completed/Clean`，且含 eBPF/network 证据；
- 删除 VM A 后 VM B 仍运行；
- 测试只清理本轮拥有的 task/container。

该用例最大同时运行 2 台 VM，每台 data Profile 至少需要 2 个 vCPU。缺少 xiaoO 或
外部虚拟化条件时该用例为 `SKIPPED`；manifest 过期或隔离断言失败为 `FAILED`。

### 详细说明

- [V2 拓扑、准备、证据与排障](v2/README.zh.md)
- [基础虚机容器矩阵](../virtual_container/README.zh.md)
- [Kata 部署与支持边界](../../../../deploy/virtual-container/README.md)

普通 Docker 的 `multi-container-xiaoo` 测试验证的是宿主容器并发；它不会启动 Kata
VM，也不能替代本用例的 guest 内核和 VM 生命周期隔离证据。

### 场景配置与运行环境

本功能在两台彼此独立的 Kata VM 中同时运行两个 xiaoO workload，验证真实并发、
观测数据隔离和 VM 生命周期隔离。公共 case 名称为
`virtual_container_xiaoo_concurrency`。

每台 VM 内都有自己的 guest-root `actraild`、openEuler workload、本地 Provider 和
xiaoO。Provider 只监听该 workload 的 loopback，因此不需要外网、CNI 或真实模型
Token。

需要同时验证镜像内的 OpenCode 时，显式设置一个当前可用的 OpenCode 免费模型：

```bash
VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_OPENCODE_FREE_MODEL=\
opencode/mimo-v2.5-free \
  deploy/virtual-container/host/run-v2-tests.sh \
    --profile local/kata/v2-test-profile.json \
    --case virtual_container_xiaoo_concurrency \
    --color never
```

启用后，每台 xiaoO workload 还会在同一个 AcTrail trace 进程树内运行一次
`opencode run --pure`。用例只接受 `opencode/*-free`，并为 OpenCode 创建全新的临时
HOME/XDG 目录，不挂载也不读取宿主 `auth.json`。无 CNI 环境中，workload 内的
OpenCode 通过专用 VSOCK `43181` 连接 Host 上仅允许 `CONNECT opencode.ai:443` 和
`CONNECT models.opencode.ai:443` 的临时代理；代理随 case 启停，其他目标返回 `403`。
镜像预置同版本 `@opencode-ai/plugin` 和构建时最新的模型目录缓存，并复制进隔离
HOME；冷启动不访问 npm registry，也不重新下载完整模型目录。该开关默认关闭，因此不会改变原有
无网、无 key 的确定性回归基线。当前这条可选出境只实现 StratoVirt；Cloud
Hypervisor 选择该开关会在启动 VM 前明确拒绝。
xiaoO 基线仍使用本地 Provider；真实 xiaoO provider key 只允许在单独的交互 smoke
中通过隐藏输入或临时环境变量传入，不写 profile、镜像或日志。
