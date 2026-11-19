# Container Auto V2

## 测试目标

验证容器启动时自动选择可用采集权限，并确保 trace 和注入通道遵守容器身份边界。

## 场景设计

在长驻 Docker 容器中执行 HTTPS 请求，覆盖 host eBPF 与 seccomp notify 的四种开关组合。通过其他容器访问目标 trace 和注入通道，再请求不可用的 required 权限，核对隔离与失败行为。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储和查询观测记录 |
| 主机 | Linux、root、可工作的 eBPF 环境 | 采集容器活动 |
| Docker | 可访问的 daemon 和测试镜像 | 运行隔离容器 |

## 测试步骤摘要

1. 检查主机与 Docker 条件，准备测试镜像。
2. 刷新默认配置并应用对应场景补丁，启动 daemon。
3. 依次运行四种权限组合，保存各次请求所属 trace。
4. 尝试跨容器查询、删除 trace 和注入事件。
5. 请求不可用的 required 权限，再恢复 host eBPF。
6. 核对观测、拒绝证据和 daemon 状态，清理本轮资源。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 权限选择 | 分别开启或关闭 host eBPF 和 seccomp notify | 启动结果与可用权限组合一致 |
| 采集内容 | 容器完成 HTTPS 请求 | TLS payload 属于对应 trace，eBPF 和 process-seccomp 事件符合开关组合 |
| 身份与注入隔离 | 其他容器访问目标 trace 或伪造登记凭据 | 访问被拒绝并有审计证据，伪造 payload 入库数为零 |
| required 权限 | 请求的权限不可用 | 启动失败并说明原因 |
| 恢复 | 矩阵完成后重新启用 host eBPF | daemon 状态正常 |

## 快速运行

在仓库根目录执行：

```bash
sudo -E python3 tests/e2e/regression/container_auto/v2/run_e2e.py \
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

## 手动测试

### 配置

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

### 权限矩阵与详细验收

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
  拒绝响应及诊断证据。`fixtures/tls_injection.py` 先发送携带格式正确的伪造凭据的
  TLS-sync 登记帧，再向目标 trace 注入 payload；要求出现对应 Trace 的鉴权拒绝审计，
  并确认伪造 payload 入库数为 0。
- **权限失败**：请求不可用的 `required` 权限时，启动返回非零退出状态和明确原因。
- **恢复验证**：矩阵结束后重新启用 host eBPF，并检查 daemon 状态。
- **清理**：结束时清理本轮容器、daemon 和随机运行目录，保留缓存镜像。
