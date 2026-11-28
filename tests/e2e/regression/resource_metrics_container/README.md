# 容器资源指标回归

## 测试目标

验证 Docker 容器资源计数器的采集、只读访问、重启恢复和退出收尾。

## 场景设计

使用已有本地镜像启动容器工作负载，在容器运行、daemon 重启和容器退出阶段核对资源记录及收尾结果。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储与查询 |
| 主机 | Linux、root、cgroup v2 | 读取容器资源计数器 |
| Docker | 可访问的 daemon | 运行容器 |
| 测试镜像 | 本地已有，包含 /bin/sh 和 sleep | 执行容器工作负载 |

## 测试步骤摘要

1. 检查 cgroup、Docker 和指定镜像，缺少运行条件时跳过。
2. 启动容器并采集资源计数器。
3. 核对采集对容器控制文件的只读行为。
4. 重启采集 daemon，检查恢复后的资源记录。
5. 结束容器，核对退出收尾并清理本轮资源。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 计数器采集 | 容器工作负载运行 | 产生对应容器的资源记录 |
| 只读控制 | 采集器读取 cgroup 计数器 | 容器资源控制值保持原值 |
| 重启恢复 | daemon 重启 | 恢复容器资源采集 |
| 退出收尾 | 容器退出 | 资源采集完成收尾 |

## 快速运行

```bash
RESOURCE_METRICS_CONTAINER_E2E_IMAGE=your-local-image \
  python3 tests/e2e/regression/test_all.py --case resource_metrics_container
```

镜像需要提供 `/bin/sh` 和 `sleep`。环境要求 root、可访问的 Docker daemon、
cgroup v2 和 release 二进制。镜像由 `RESOURCE_METRICS_CONTAINER_E2E_IMAGE`
指定，缺少运行条件时报告跳过。工作目录与日志由统一 runner 管理。
