# 虚拟容器 xiaoO 并发回归测试

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

## 快速运行

部署 manifest 中已经包含 xiaoO 后执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container_xiaoo_concurrency \
  --color never
```

两个虚拟容器 case 一键运行：

```bash
deploy/virtual-container/host/run-v2-tests.sh --color never
```

同一轮中基础 `virtual_container` 为 `SKIPPED` 时，并发 case 同样为 `SKIPPED`；
单独选择并发 case 时仍独立检查 KVM、artifact 和 xiaoO。

## 核心通过条件

- 两台 Kata VM 同时处于 Ready；
- 两个 Provider Ready 后统一释放 barrier；
- 两个 xiaoO 进程存在同一活跃窗口；
- 启用 OpenCode smoke 时，两边免费模型均返回各自的响应 marker；
- A/B 返回值、文件 marker 和 trace 不串线；
- 两条 trace 均为 `Completed/Clean`，且含 eBPF/network 证据；
- 删除 VM A 后 VM B 仍运行；
- 测试只清理本轮拥有的 task/container。

该用例最大同时运行 2 台 VM，每台 data Profile 至少需要 2 个 vCPU。缺少 xiaoO 或
外部虚拟化条件时该用例为 `SKIPPED`；manifest 过期或隔离断言失败为 `FAILED`。

## 详细说明

- [V2 拓扑、准备、证据与排障](v2/README.zh.md)
- [基础虚拟容器矩阵](../virtual_container/README.zh.md)
- [Kata 部署与支持边界](../../../../deploy/virtual-container/README.md)

普通 Docker 的 `multi-container-xiaoo` 测试验证的是宿主容器并发；它不会启动 Kata
VM，也不能替代本用例的 guest 内核和 VM 生命周期隔离证据。
