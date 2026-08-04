# Container Auto V2

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/container_auto/v2/run_e2e.py
```

该脚本模拟普通用户把 AcTrail Agent 部署进非特权 Docker 容器：容器先以
`tail -f /dev/null` 长驻，再通过 `docker exec` 执行
`actrailctl launch --host-ebpf auto --seccomp-notify auto`。测试覆盖 host eBPF 与
seccomp notify 的四种组合，并验证容器隔离和 `required` 权限失败语义。
`sudo -E` 保留调用者已经导出的环境，runner 及其 Docker 子进程继续继承。

# 步骤摘要

1. 检查 release 二进制和 Docker daemon。
2. 按 Dockerfile、基础镜像、`actrailctl` 和 TLS probe 的内容生成镜像标签；已存在
   的同标签镜像直接复用。
3. 验证 `host eBPF enabled + seccomp notify enabled`。
4. 验证 `host eBPF enabled + seccomp notify disabled`。
5. 验证 `host eBPF disabled + seccomp notify enabled`。
6. 验证 `host eBPF disabled + seccomp notify disabled`。
7. 验证不同容器不能查看、删除或向彼此的 trace 注入 seccomp/TLS 数据。
8. 验证请求 `required` 但权限不可用时明确失败。
9. 恢复 `ebpf = "auto"`，确认 daemon 能重新启用 host eBPF。

# 手动测试

以下命令均从仓库根目录执行。手动测试使用固定名称，执行前先设置公共变量：

```bash
CASE_DIR="tests/v2/regression/container_auto/v2"
BIN_DIR="${ACTRAIL_BIN_DIR:-target/release}"
BASE_IMAGE="${CONTAINER_AUTO_E2E_BASE_IMAGE:-ubuntu:24.04}"
RUNTIME_DIR="$(mktemp -d /tmp/actrail-container-auto-manual.XXXXXX)"
AUTO_IMAGE="actrail/container-auto-v2:manual"
mkdir -p "$RUNTIME_DIR/image" "$RUNTIME_DIR/run" "$RUNTIME_DIR/data/export" \
  "$RUNTIME_DIR/log" "$RUNTIME_DIR/etc/actrail/plugins/otel-jsonl"
```

## 步骤1：检查测试前提

### 手动指令

```bash
test "$(id -u)" -eq 0
test -x "$BIN_DIR/actraild"
test -x "$BIN_DIR/actrailctl"
test -x "$BIN_DIR/libactrail_tls_payload_probe_sync.so"
test -f "$CASE_DIR/Dockerfile"
test -f "$CASE_DIR/operator.conf"
test -f "$CASE_DIR/seccomp/actrail-notify.json"
docker info --format '{{.ServerVersion}}'
```

### 脚本行为与预期结果

缺少 AcTrail release 产物属于 `FAILED`；Docker 命令或 daemon 不可用属于外部前提
不满足，测例标记为 `SKIPPED`。所有检查通过后才允许创建运行时资源。

## 步骤2：准备可复用镜像

### 手动指令

```bash
cp "$BIN_DIR/actrailctl" "$RUNTIME_DIR/image/actrailctl"
cp "$BIN_DIR/libactrail_tls_payload_probe_sync.so" \
  "$RUNTIME_DIR/image/libactrail_tls_payload_probe_sync.so"
docker image inspect "$AUTO_IMAGE" >/dev/null 2>&1 || \
  docker build -q \
    -f "$CASE_DIR/Dockerfile" \
    --build-arg "BASE_IMAGE=$BASE_IMAGE" \
    -t "$AUTO_IMAGE" \
    "$RUNTIME_DIR/image"
docker image inspect --format '{{.RepoTags}}' "$AUTO_IMAGE"
```

### 脚本行为与预期结果

自动脚本不使用固定 `manual` 标签，而是计算内容摘要并生成
`actrail/container-auto-v2:<content-hash>`。本地已有该标签时不执行
`docker build`，固定输入改变或设置 `CONTAINER_AUTO_E2E_REBUILD_IMAGE=1` 时才重建。
镜像一次性安装 `curl`、Python、证书和 `tini`，每轮测试不再刷新这些固定依赖。

## 步骤3：启动 host eBPF 可用的隔离 daemon

### 手动指令

```bash
cp examples/plugins/builtin/otel-jsonl/otel-jsonl.plugin.toml \
  examples/plugins/builtin/otel-jsonl/otel-jsonl.config.v1.schema.json \
  "$RUNTIME_DIR/etc/actrail/plugins/otel-jsonl/"
sed "s|/var/lib/actrail|$RUNTIME_DIR/data|g" \
  examples/plugins/builtin/otel-jsonl/otel-jsonl.config.toml \
  > "$RUNTIME_DIR/etc/actrail/plugins/otel-jsonl/otel-jsonl.config.toml"
sed \
  -e "s|@RUNTIME_DIR@|$RUNTIME_DIR|g" \
  -e 's|@EBPF_ENABLED@|"auto"|g' \
  "$CASE_DIR/operator.conf" > "$RUNTIME_DIR/container-auto.conf"
"$BIN_DIR/actraild" --config "$RUNTIME_DIR/container-auto.conf" run \
  > "$RUNTIME_DIR/log/actraild.stderr" 2>&1 &
DAEMON_PID=$!
until test -S "$RUNTIME_DIR/run/control.sock"; do sleep 0.1; done
"$BIN_DIR/actrailctl" --config "$RUNTIME_DIR/container-auto.conf" doctor | rg ebpf
```

### 脚本行为与预期结果

daemon 使用本轮独立 socket、SQLite 和日志目录。`doctor` 必须报告 eBPF collector；
两个要求 host eBPF enabled 的 Case 只有在宿主确实提供 collector 时才继续。

## 步骤4：host eBPF enabled + seccomp notify enabled

### 手动指令

```bash
docker run -d --name actrail-auto-both-manual --user 0:0 \
  --security-opt "seccomp=$PWD/$CASE_DIR/seccomp/actrail-notify.json" \
  -v "$RUNTIME_DIR:$RUNTIME_DIR:ro" \
  "$AUTO_IMAGE" tail -f /dev/null
docker exec actrail-auto-both-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf auto --seccomp-notify auto -- \
  /bin/sh -c 'curl -sS https://example.com/ -o /dev/null'
```

### 脚本行为与预期结果

launch 必须输出 `host_ebpf:enabled,seccomp_notify:enabled`，trace profile 为
`container-auto-ebpf-on-notify-on`。trace 必须包含 TLS payload、eBPF event 和
`process-seccomp` event；容器必须非 privileged、非 host PID、无额外 capability。

## 步骤5：host eBPF enabled + seccomp notify disabled

### 手动指令

```bash

docker run -d --name actrail-auto-host-only-manual --user 0:0 \
  -v "$RUNTIME_DIR:$RUNTIME_DIR:ro" \
  "$AUTO_IMAGE" tail -f /dev/null
docker exec actrail-auto-host-only-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf auto --seccomp-notify auto -- \
  /bin/sh -c 'curl -sS https://example.com/ -o /dev/null'
```

### 脚本行为与预期结果

launch 必须输出 `host_ebpf:enabled,seccomp_notify:disabled`，trace profile 为
`container-auto-ebpf-on-notify-off`。trace 必须包含 TLS payload 和 eBPF event，
不得包含 `process-seccomp` event；容器权限约束与步骤4相同。

## 步骤6：关闭 host eBPF

### 手动指令

```bash
kill "$DAEMON_PID"
wait "$DAEMON_PID" || true
sed \
  -e "s|@RUNTIME_DIR@|$RUNTIME_DIR|g" \
  -e 's|@EBPF_ENABLED@|false|g' \
  "$CASE_DIR/operator.conf" > "$RUNTIME_DIR/container-auto.conf"
"$BIN_DIR/actraild" --config "$RUNTIME_DIR/container-auto.conf" run \
  >> "$RUNTIME_DIR/log/actraild.stderr" 2>&1 &
DAEMON_PID=$!
until test -S "$RUNTIME_DIR/run/control.sock"; do sleep 0.1; done
! "$BIN_DIR/actrailctl" --config "$RUNTIME_DIR/container-auto.conf" doctor | rg ebpf
```

### 脚本行为与预期结果

daemon 用同一份隔离 runtime 重启；`doctor` 不得再报告 eBPF collector。每个
eBPF-disabled Case 都会先确保该状态，因此四个 Case 改变顺序或反向执行仍成立。

## 步骤7：host eBPF disabled + seccomp notify enabled

### 手动指令

```bash

docker run -d --name actrail-auto-notify-only-manual --user 0:0 \
  --security-opt "seccomp=$PWD/$CASE_DIR/seccomp/actrail-notify.json" \
  -v "$RUNTIME_DIR:$RUNTIME_DIR:ro" "$AUTO_IMAGE" tail -f /dev/null
docker exec actrail-auto-notify-only-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf auto --seccomp-notify auto -- /bin/true
```

### 脚本行为与预期结果

launch 必须输出 `host_ebpf:disabled,seccomp_notify:enabled`，trace profile 为
`container-auto-ebpf-off-notify-on`。trace 必须包含 `process-seccomp` event，不得
包含 eBPF event。

## 步骤8：host eBPF disabled + seccomp notify disabled

### 手动指令

```bash

docker run -d --name actrail-auto-neither-manual --user 0:0 \
  -v "$RUNTIME_DIR:$RUNTIME_DIR:ro" "$AUTO_IMAGE" tail -f /dev/null
docker exec actrail-auto-neither-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf auto --seccomp-notify auto -- /bin/true
```

### 脚本行为与预期结果

launch 必须输出 `host_ebpf:disabled,seccomp_notify:disabled`，trace profile 为
`container-auto-ebpf-off-notify-off`。trace 不得包含 eBPF 或
`process-seccomp` event。

## 步骤9：验证跨容器隔离和 required 失败

### 手动指令

```bash
docker exec actrail-auto-neither-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" list-traces

! docker exec actrail-auto-neither-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf required --seccomp-notify disabled -- /bin/true

! docker exec actrail-auto-neither-manual \
  actrailctl --config "$RUNTIME_DIR/container-auto.conf" launch \
  --host-ebpf disabled --seccomp-notify required -- /bin/true
```

### 脚本行为与预期结果

自动脚本另外创建 A、B 和一个 `--pid=host` 长驻容器。B 与 host-PID 容器均不得
列出或删除 A 的活动 trace；伪造 seccomp listener 和 TLS-sync payload 必须因
`peer_identity` 被拒绝，且数据库中不能出现伪造 payload。两条 `required` 命令
必须非零退出，并分别包含 `host eBPF required` 和 `seccomp-notify required`。

## 步骤10：清理

### 手动指令

```bash
docker rm -f \
  actrail-auto-both-manual \
  actrail-auto-host-only-manual \
  actrail-auto-notify-only-manual \
  actrail-auto-neither-manual
kill "$DAEMON_PID"
wait "$DAEMON_PID" || true
rm -rf "$RUNTIME_DIR"
```

### 脚本行为与预期结果

自动脚本只删除带本轮所有权的容器和临时 runtime；内容寻址镜像保留，供下一次
回归直接复用。连续运行两次 Quick Run，第二次应显示相同镜像引用且不重新构建。
