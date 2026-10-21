# Container Auto V2

该测例验证 Docker 容器内 `actrailctl launch` 对 host eBPF 和 seccomp notify
权限的自动选择，以及跨容器 trace 隔离和 `required` 权限失败行为。

## 运行

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/container_auto/v2/run_e2e.py \
  --bin-dir target/release
```

- **运行前提**：需要 Docker daemon、root 权限、支持 eBPF 的宿主内核，以及
  `actraild`、`actrailctl`、`actrailviewer` 和 TLS-sync 运行库的 release 产物。
- **基础镜像**：`CONTAINER_AUTO_E2E_BASE_IMAGE` 指定镜像，默认
  `openeuler/openeuler:24.03-lts-sp3`。镜像包含 curl、Python、证书和容器 init。
- **环境继承**：`sudo -E` 保留调用者导出的环境变量。`TARGET_URL` 指定 HTTPS
  请求地址，默认 `https://example.com/`。
- **时间限制**：`CONTAINER_AUTO_E2E_TIMEOUT_SECONDS` 默认 1200 秒，
  `CONTAINER_AUTO_E2E_CLEANUP_GRACE_SECONDS` 默认 30 秒。
- **镜像复用**：镜像标签由基础镜像、Dockerfile、启动器和 TLS-sync 运行库内容生成。
  `CONTAINER_AUTO_E2E_REBUILD_IMAGE=1` 强制重建。准备镜像时清理同仓库的其他旧标签。

## 配置

- **运行路径**：每轮创建随机 `/tmp/actrail-container-auto-v2.*` 目录。公共
  `PatchBuilder` 将 control socket、数据库、日志、导出目录和插件路径写入
  `runtime.patch.toml`。
- **场景行为**：本目录的 `patch_config.toml` 声明采集能力、保留策略和
  `execve`、`execveat` 的进程执行观测。TLS 后端继承产品默认配置。
- **无 eBPF 专项**：`tls-only/patch_config.toml` 关闭 eBPF 并选择 TLS-sync。
  它在公共场景补丁之后应用。
- **插件输出**：`PatchBuilder` 读取 OTEL JSONL 插件配置，并将输出路径设置为
  本轮运行目录下的 `data/export/live-spans.otlp.jsonl`。

启动器从自己的文件位置定位场景补丁。每次 daemon 启动先刷新默认配置，再按命令
顺序应用补丁。后面的补丁覆盖前面的同名字段。

```bash
"$BIN_DIR/actrailctl" --config "$RUNTIME_DIR/container-auto.conf" init -f \
  --patch "$RUNTIME_DIR/runtime.patch.toml" \
  --patch "$CASE_DIR/patch_config.toml"
```

无 eBPF 专项增加最后一个补丁：

```bash
"$BIN_DIR/actrailctl" --config "$RUNTIME_DIR/container-auto.conf" init -f \
  --patch "$RUNTIME_DIR/runtime.patch.toml" \
  --patch "$CASE_DIR/patch_config.toml" \
  --patch "$CASE_DIR/tls-only/patch_config.toml"
```

以上命令中的 `BIN_DIR`、`RUNTIME_DIR` 和 `CASE_DIR` 分别对应本轮 release
目录、随机运行目录和测例目录。`runtime.patch.toml` 由场景准备过程生成。

## 验收

容器先以 `tail -f /dev/null` 长驻，再通过 `docker exec` 启动
`actrailctl launch --host-ebpf auto --seccomp-notify auto -- /usr/bin/curl`。
请求的 TLS 内容写入所属 trace。容器使用独立 PID 命名空间和默认 capability 集合。

| host eBPF | seccomp notify | TLS 后端 | 验收内容 |
|---|---|---|---|
| 开启 | 开启 | 产品默认 | 正确权限组合、TLS payload、eBPF 事件 |
| 开启 | 关闭 | 产品默认 | 正确权限组合、TLS payload、eBPF 事件、零 process-seccomp 事件 |
| 关闭 | 开启 | TLS-sync | 正确权限组合、TLS payload、零 eBPF 事件 |
| 关闭 | 关闭 | TLS-sync | 正确权限组合、TLS payload、零 eBPF 和 process-seccomp 事件 |

- **启动就绪**：所有场景等待 control socket，TLS-sync 场景同时等待事件 socket。
- **身份隔离**：其他容器无法列出或删除目标容器的活动 trace。host-PID 容器仍需
  通过容器身份校验。
- **注入隔离**：跨容器注册 seccomp listener 或注入 TLS-sync payload 时，检查
  拒绝响应及诊断证据。TLS 数据帧由 `fixtures/tls_injection.py` 构造。
- **权限失败**：请求不可用的 `required` 权限时，启动返回非零退出状态和明确原因。
- **恢复验证**：矩阵结束后重新启用 host eBPF，并检查 daemon 状态。
- **清理**：结束时清理本轮容器、daemon 和随机运行目录，保留缓存镜像。
