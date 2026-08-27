# 普通容器部署 V2 回归

该测例通过 V2 公共 runner 调用现有
`deploy/container-auto/e2e.sh`，验证 Docker 普通容器部署的完整权限矩阵、
跨容器控制隔离和并发归属。

## 运行

先构建 release 产物，然后以 root 运行：

```bash
cargo build --release
sudo -E python3 tests/v2/regression/test_all.py \
  --case container_auto \
  --no-cleanup
```

也可以单独运行并显示完整明细：

```bash
sudo -E python3 tests/v2/regression/container_auto/run_e2e.py \
  --no-cleanup
```

外部前置条件包括可用的 Docker daemon 和 `sqlite3`。缺少外部环境时测例显示
跳过；AcTrail release 产物缺失或验收脚本断言失败时测例失败。

环境变量：

- `CONTAINER_AUTO_E2E_TIMEOUT_SECONDS`：完整验收超时，默认 1200 秒。
- `CONTAINER_AUTO_E2E_CLEANUP_GRACE_SECONDS`：超时后留给脚本清理的时间，
  默认 30 秒。
- `CONTAINER_AUTO_E2E_BASE_IMAGE`：验收使用的基础镜像，默认
  `openeuler/openeuler:24.03-lts-sp3`；验证 Ubuntu 时设为 `ubuntu:24.04`，公共
  registry 不可用时可指定对应发行版的镜像源地址。

原验收脚本始终通过唯一运行标签清理它创建的容器、镜像和临时目录。因此
`--no-cleanup` 会保留 V2 runner 日志，但不会保留这些 Docker 临时资源。
