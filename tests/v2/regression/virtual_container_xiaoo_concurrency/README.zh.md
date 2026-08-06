# 虚拟容器 xiaoO 并发回归测试

本功能在两台彼此独立的 Kata VM 中同时运行两个 xiaoO workload，验证真实并发、
观测数据隔离和 VM 生命周期隔离。公共 case 名称为
`virtual_container_xiaoo_concurrency`。

每台 VM 内都有自己的 guest-root `actraild`、openEuler workload、本地 Provider 和
xiaoO。Provider 只监听该 workload 的 loopback，因此不需要外网、CNI 或真实模型
Token。

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
