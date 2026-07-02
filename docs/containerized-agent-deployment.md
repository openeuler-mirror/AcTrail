# 容器化 Agent 部署

## 文档目标

本文档说明如何在主机侧运行 AcTrail daemon、viewer、web 和 SQLite 存储，同时在一个 Docker workload 容器内运行被观测的 Agent，并通过容器内的 `actrailctl launch` 让主机 daemon 采集容器内 Agent 的进程、网络、TLS 明文载荷和语义动作。

本文档的默认路径是“主机 + 一个已有 workload 容器”：主机负责采集和存储，容器只负责运行 `actrailctl launch -- <agent>` 和实际 Agent 进程。

容器化部署通过 `--host-ebpf` 和 `--seccomp-notify` 两个独立权限轴，
根据宿主机与容器的实际能力选择采集 profile。`auto` 会在能力不可用时
明确降级，`required` 会直接失败，`disabled` 则保证不启用对应采集机制。

> 权限矩阵、完整部署资产、seccomp profile 和验收测试见
> [Container Permission Auto-Selection](../deploy/container-auto/README.md)；
> 本文只说明单机 Docker 主流程。

## 说明什么东西

本文档说明的是 AcTrail 在容器化 Agent 场景下的运行部署，不说明如何把 `actraild` 本身放进容器，也不说明如何通过 TCP 转发 AcTrail 控制 socket。

运行时只有一个被观测 workload 容器；主机上的 `actraild`、`actrailviewer`、`actrailweb` 不是容器；如果你额外使用 build 容器编译 release 产物，它也不属于这次被观测的运行拓扑。

主机侧组件负责 eBPF、seccomp、TLS-sync、PID namespace 映射、SQLite 写入和 semantic action 投影。容器侧组件只需要 `actrailctl`、`libactrail_tls_payload_probe_sync.so`、Agent 二进制和 Agent 自己的配置/密钥。

推荐运行拓扑如下：

```text
host actraild
  -> host eBPF/seccomp/TLS-sync collector
  -> host semantic action runtime
  -> host /var/lib/actrail/actrail.sqlite
  -> host actrailviewer / actrailweb

Docker workload container
  -> actrailctl launch -- /path/to/agent ...
  -> child agent process in container PID namespace
  -> /run/actrail/control.sock and /run/actrail/tls-sync.sock mounted from host
```

### Docker 权限模式简述

| Docker seccomp 模式 | `--seccomp-notify auto` 的结果 | 用途 |
| --- | --- | --- |
| 默认 profile | notify 不可用时自动降级，保留 TLS-sync | 最小权限部署 |
| `actrail-notify.json` | 启用 notify，同时保留 Docker 外层过滤 | 推荐的完整采集部署 |
| `seccomp=unconfined` | 启用 notify，但关闭 Docker 外层过滤 | 可信测试、兼容性排障或明确接受更宽 syscall 面的环境 |

三种方式都受支持。正常部署优先使用默认 profile 自动降级，或使用
`actrail-notify.json` 获得完整采集；详细选择规则见专门部署文档。

## 整体流程

通过在主机启动 `actraild`，把主机 `/run/actrail` socket 目录挂载到 workload 容器，在容器内执行 `actrailctl launch -- xiaoo --cli run -p "你好"`，然后回到主机侧检查 `actrailviewer`、`actrailweb` 和 `/var/lib/actrail/actrail.sqlite`，验证容器内 Agent 是否被完整观测。

### 前提假设

- 主机已经能用 release 版本 AcTrail 组件，至少包括 `actraild`、`actrailctl`、`actrailviewer`、`actrailweb` 和 `libactrail_tls_payload_probe_sync.so`。
- 主机默认配置路径是 `/etc/actrail/actraild.conf`，默认 socket 目录是 `/run/actrail`，默认 SQLite 路径是 `/var/lib/actrail/actrail.sqlite`。
- 主机 `actraild` 配置启用了 TLS plaintext，并使用 TLS-sync 后端；如果不满足，先看“其他分支情况 5：主机配置没有启用 TLS-sync”。
- 已有 workload 容器名通过 `AGENT_CONTAINER` 指定，并且这个容器里已经有 xiaoO、模型密钥、xiaoO 配置、`actrailctl` 和 `libactrail_tls_payload_probe_sync.so`；如果缺少 AcTrail 组件，先看“其他分支情况 2：容器内没有 AcTrail release 组件”。
- workload 容器创建时已经挂载 `/run/actrail`；按上表选择 Docker seccomp 模式，并用 `actrailctl probe` 检查实际结果；最小配置示例见 [container-agent-minimal](examples/container-agent-minimal/README.md)。
- xiaoO 默认路径按 `/root/.cargo/bin/xiaoo` 和 `/root/api_key.sh` 书写；如果你的容器内路径不同，按“其他分支情况 3：Agent 路径、密钥或代理不同”调整。

### 具体步骤

#### 1. 在兼容构建环境编译 release 组件

操作：

```bash
cargo fmt --all
cargo build --release -p daemon -p ctl -p view -p web -p tls_payload_probe_sync
```

说明：这一步生成运行所需的 release 二进制；`actraild` 负责采集，`actrailviewer` 和 `actrailweb` 负责验证，`tls_payload_probe_sync` 是 TLS-sync 载荷捕获组件。构建机可以不是部署服务器,但构建产物要求的 GLIBC 版本必须不高于服务器和 Agent 基础镜像提供的版本。若构建机更新,应在与目标系统兼容的构建容器/虚拟机中编译,再上传预编译产物;部署服务器本身不需要 cargo/clang。

预期结果：`target/release/actraild`、`target/release/actrailctl`、`target/release/actrailviewer`、`target/release/actrailweb` 和 `target/release/libactrail_tls_payload_probe_sync.so` 存在。上传后应先在目标环境运行 `actrailctl --help`、`actrailctl probe --help` 和 `ldd` 检查,确认二进制来自当前源码且没有 `GLIBC_2.xx not found`/缺失动态库。

#### 2. 在主机启动 `actraild`

操作：

```bash
./target/release/actraild --config /etc/actrail/actraild.conf start
./target/release/actrailctl --config /etc/actrail/actraild.conf doctor
```

说明：这一步让主机 daemon 创建 control socket 和 TLS-sync socket，并确认 collector、storage 和运行平台能力可用。

预期结果：`doctor` 成功返回，主机上存在 `/run/actrail/control.sock` 和 `/run/actrail/tls-sync.sock`，默认 SQLite 数据库路径 `/var/lib/actrail/actrail.sqlite` 可由 daemon 写入。

#### 3. 指定并检查 workload 容器

操作：

```bash
printf '输入已有 workload 容器名: '
read -r AGENT_CONTAINER
test -n "$AGENT_CONTAINER"
docker exec "$AGENT_CONTAINER" test -S /run/actrail/control.sock
docker exec "$AGENT_CONTAINER" test -S /run/actrail/tls-sync.sock
docker exec "$AGENT_CONTAINER" test -x /usr/local/bin/actrailctl
docker exec "$AGENT_CONTAINER" test -x /usr/local/bin/libactrail_tls_payload_probe_sync.so
docker exec "$AGENT_CONTAINER" test -x /root/.cargo/bin/xiaoo
```

说明：这一步确认容器能访问主机 AcTrail socket，并确认容器内有 launch 所需的 `actrailctl`、TLS-sync preload library 和 xiaoO。

预期结果：所有 `test` 命令退出码都是 0；如果 `/run/actrail/*.sock` 不存在，说明容器启动时没有正确挂载主机 socket 目录；如果 `actrailctl` 或 `.so` 不存在，说明容器侧 release 组件还没有安装。

#### 3.1 在容器内运行 `actrailctl probe`

操作：

```bash
docker exec "$AGENT_CONTAINER" \
  actrailctl --config /etc/actrail/actraild.conf probe \
    --host-ebpf auto \
    --seccomp-notify auto
```

说明：这一步在 launch 前检查容器内能否访问 control/TLS-sync socket、no_new_privs、seccomp-notify launch 路径和 TLS-sync runtime library。ctl 把容器内的 seccomp 探测结果发送给 daemon；daemon 再结合自己的 host eBPF collector、配置和两个权限轴生成最终不可变 profile。`--skip-daemon` 只提供本地预览，不是 launch 的最终权限决策。

预期结果：human 输出包含 `deployment_permissions_requested`、`deployment_permissions_selected`、`deployment_permissions_degraded` 和最终 required capabilities；发生降级或配置裁剪时还会包含可机器解析的原因。JSON 输出包含相同的 `deployment_permissions` 对象。

control 和 TLS-sync socket 在 accept 后读取内核 `SO_PEERCRED`。创建 trace
的 container principal 会绑定到该 trace；另一个即使挂载了同一
`/run/actrail` 的容器，也不能 remove/register 该 trace 或向它注入 TLS
event。拒绝错误码固定为 `peer_identity`，daemon journald 中同时记录
`actrail::peer_auth` 审计信息。host root 保留运维权限，普通 host uid 仍受
socket 文件权限和 peer uid 校验约束。

JSON 示例：

```bash
docker exec "$AGENT_CONTAINER" \
  actrailctl --config /etc/actrail/actraild.conf probe \
    --host-ebpf auto --seccomp-notify auto --json
```

#### 4. 检查 workload 容器的 Docker 运行选项

操作：

```bash
docker inspect "$AGENT_CONTAINER" --format '{{json .HostConfig.SecurityOpt}}'
docker inspect "$AGENT_CONTAINER" --format '{{range .Mounts}}{{println .Destination "<-" .Source}}{{end}}'
```

说明：这一步确认容器使用的是 Docker 默认、AcTrail 定制还是 `unconfined` seccomp 模式，并确认 `/run/actrail` 和 `/etc/actrail` 是否从主机挂载进容器。

预期结果：`SecurityOpt` 包含定制 profile（`actrail-notify.json`）或 `seccomp=unconfined` 时，seccomp-notify 轴可用；正常部署推荐定制 profile，`unconfined` 仅用于可信测试、兼容性排障或明确接受更宽 syscall 面的环境。如果仍是 Docker 默认 profile，不一定阻塞 launch——`--seccomp-notify auto` 会自动降级，先看第 3.1 步 `probe` 的 `deployment_permissions_selected`。挂载列表里应能看到 `/run/actrail <- /run/actrail`，最好也能看到 `/etc/actrail <- /etc/actrail` 且只读；如果缺少 socket 挂载，不能靠 `docker exec` 补挂载，必须参考“其他分支情况 1”保留现有容器并新建一个带正确选项的 workload 容器。

#### 5. 设置 Agent 出网代理

操作：

```bash
export AGENT_PROXY=http://172.17.0.1:8118
```

说明：这一步只给 Agent 的模型访问链路准备 HTTP/HTTPS 代理；AcTrail control 和 TLS-sync 使用 Unix socket，不经过这个 HTTP proxy。

预期结果：后续 `docker exec` 可以通过 `-e HTTP_PROXY="$AGENT_PROXY"` 等环境变量让 xiaoO 使用主机 Privoxy 出网，并且 `NO_PROXY` 仍然包含 `localhost,127.0.0.1,::1`。

#### 6. 在容器内通过 `actrailctl launch` 启动 xiaoO

操作：

```bash
docker exec \
  -e HTTP_PROXY="$AGENT_PROXY" \
  -e HTTPS_PROXY="$AGENT_PROXY" \
  -e http_proxy="$AGENT_PROXY" \
  -e https_proxy="$AGENT_PROXY" \
  -e NO_PROXY=localhost,127.0.0.1,::1 \
  -e no_proxy=localhost,127.0.0.1,::1 \
  "$AGENT_CONTAINER" \
  bash -lc 'source /root/api_key.sh && export PATH=/usr/local/bin:/root/.cargo/bin:$PATH && actrailctl --config /etc/actrail/actraild.conf launch --host-ebpf auto --seccomp-notify auto -- /root/.cargo/bin/xiaoo --cli run -p "你好"'
```

说明：这一步在已有 workload 容器内启动被观测 Agent。`host-ebpf × seccomp-notify` 根据真实权限选择四种不可变 profile snapshot。把任一轴设为 `required` 会在权限不可用时 fail-loud；设为 `disabled` 会保证该 trace 不绑定对应机制。新部署使用 `--host-ebpf`/`--seccomp-notify` 两个权限轴。完整规则见 [Container Permission Auto-Selection](../deploy/container-auto/README.md)。

预期结果：命令输出中能看到 `trace trace-<N> entered Active` 或等价 trace 启动信息，xiaoO 正常完成请求。若使用 `--seccomp-notify required` 且出现 `pidfd_getfd seccomp listener: Operation not permitted`，说明容器没提供 seccomp-notify 能力；改用 `--seccomp-notify auto` 自动降级，或按“其他分支情况 1”新建带定制 profile `actrail-notify.json` 的 workload 容器。可信测试或兼容性排障可以改用 `seccomp=unconfined`，但这会关闭 Docker 外层 syscall 过滤。

#### 7. 在主机查看 trace 摘要

操作：

```bash
./target/release/actrailctl --config /etc/actrail/actraild.conf list-traces
printf '输入刚产生的 trace id，格式为 trace-数字: '
read -r TRACE_ID
case "$TRACE_ID" in trace-[0-9]*) ;; *) echo 'trace id 格式必须是 trace-数字' >&2; exit 1 ;; esac
export TRACE_NUM="${TRACE_ID#trace-}"
./target/release/actrailviewer --config /etc/actrail/actraild.conf summary --trace-id "$TRACE_ID"
```

说明：这一步在主机侧确认容器内 xiaoO 产生的 trace 已经进入默认 SQLite，并确认 trace 生命周期、事件数量和诊断状态。

预期结果：`list-traces` 能看到刚刚启动的 trace，`TRACE_ID` 使用的是这次运行产生的 trace id，`summary` 显示 trace 已 completed 或处于合理状态，进程数量和事件数量非 0，且没有 daemon 崩溃或诊断异常。

#### 8. 在主机检查 TLS plaintext 和 semantic action

操作：

```bash
sqlite3 /var/lib/actrail/actrail.sqlite "select library, symbol, direction, count(*), sum(captured_size) from payload_segments where trace_id = $TRACE_NUM group by library, symbol, direction order by library, symbol, direction; select kind, count(*) from semantic_actions where trace_id = $TRACE_NUM group by kind order by kind;"
```

说明：这一步不直接打印敏感请求正文，只检查 TLS 明文载荷和 semantic action 类型是否存在；对于 xiaoO/rustls，关键是能看到 rustls inbound/outbound payload 和 `llm.*` action。

预期结果：`payload_segments` 至少包含 `rustls_buffer_plaintext` 和 `rustls_take_received_plaintext`，`semantic_actions` 至少包含 `llm.request`、`llm.response`、`llm.call`、`command.invocation` 或 `process.exec` 中的相关记录。

#### 9. 在主机用 `actrailweb` 验证 action tree

操作：

```bash
./target/release/actrailweb --config /etc/actrail/actraild.conf --addr 127.0.0.1 --port 18080
```

在另一个 shell 中执行：

```bash
curl -fsS "http://127.0.0.1:18080/api/traces/$TRACE_NUM/action-tree" | jq -r '[(.roots | length), (.actions | length), (.links | length)] | @tsv'
curl -fsS "http://127.0.0.1:18080/api/traces/$TRACE_NUM/commands" | jq -r 'length'
```

说明：这一步确认 web API 可以基于主机 SQLite 返回语义动作树和命令列表，证明不是只采到了进程 argv 或 stdout，而是有可用于行为分析的 action graph。

预期结果：action tree 至少有 1 个 root，actions 和 links 数量非 0，commands 接口返回数量非 0，并且能看到 xiaoO 进程和 Agent 触发的工具命令。

#### 10. 停止临时服务

操作：

```bash
./target/release/actraild --config /etc/actrail/actraild.conf stop
```

说明：这一步清理主机后台 daemon；如果你为验证临时启动了 `actrailweb`，也应结束对应前台进程或服务管理进程。

预期结果：没有遗留 `actraild` 或临时 `actrailweb` 进程，默认 SQLite 里的验证 trace 保留在 `/var/lib/actrail/actrail.sqlite` 供后续查看。

## 其他分支情况

### 1. 现有容器缺少必需 Docker 选项

如果容器没有 `/run/actrail` 挂载，或者需要 seccomp-notify 但容器仍使用 Docker 默认 profile，不能通过 `docker exec` 动态修复；不要删除这个容器，因为它的 writable layer 里可能保存了 `actrailctl`、xiaoO、密钥、配置和其他只存在于容器内的状态。若接受 `--seccomp-notify auto` 降级，则默认 profile 本身不要求重建容器。

操作：

```bash
printf '输入现有容器名: '
read -r EXISTING_AGENT_CONTAINER
test -n "$EXISTING_AGENT_CONTAINER"
export AGENT_CONTAINER="${EXISTING_AGENT_CONTAINER}-actrail"
export ACTRAIL_SECCOMP_PROFILE="$(pwd)/deploy/container-auto/seccomp/actrail-notify.json"
test -f "$ACTRAIL_SECCOMP_PROFILE"
docker run -d --name "$AGENT_CONTAINER" \
  --user 0:0 \
  --security-opt "seccomp=$ACTRAIL_SECCOMP_PROFILE" \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  openeuler/openeuler:24.03-lts-sp3 \
  tail -f /dev/null
```

说明：这一步创建一个新的替代 workload 容器，不删除 `EXISTING_AGENT_CONTAINER`；`--user 0:0` 表示容器内以 root 运行，当前验证路径需要它来完成 launch-time seccomp user notification 和 pidfd 准备；`actrail-notify.json` 在保留 Docker 外层 seccomp 过滤的同时放行 AcTrail 所需的 `pidfd_getfd`；`-v /run/actrail:/run/actrail` 是主机 daemon 与容器 ctl 通信的关键挂载。可信测试或兼容性排障可以把定制 profile 参数替换成 `--security-opt seccomp=unconfined`，但这会关闭 Docker 外层 syscall 过滤。

预期结果：旧容器仍然存在，新容器保持运行，新容器内 `test -S /run/actrail/control.sock` 和 `test -S /run/actrail/tls-sync.sock` 成功；之后需要把 xiaoO、密钥、配置和 AcTrail release 组件安装或迁移到新容器。

迁移二进制的操作：

```bash
docker exec "$EXISTING_AGENT_CONTAINER" tar -C /usr/local/bin -cf - actrailctl libactrail_tls_payload_probe_sync.so | docker exec -i "$AGENT_CONTAINER" tar -C /usr/local/bin -xf -
docker exec "$AGENT_CONTAINER" mkdir -p /root/.cargo/bin
docker exec "$EXISTING_AGENT_CONTAINER" tar -C /root/.cargo/bin -cf - xiaoo | docker exec -i "$AGENT_CONTAINER" tar -C /root/.cargo/bin -xf -
docker exec "$AGENT_CONTAINER" chmod 0755 /usr/local/bin/actrailctl /usr/local/bin/libactrail_tls_payload_probe_sync.so /root/.cargo/bin/xiaoo
```

说明：如果 `actrailctl`、TLS-sync `.so` 和 xiaoO 二进制只存在于旧容器，可以用 tar 管道迁移这些明确需要的文件；不要复制整个 `/root`、整个仓库或其他未知目录。

预期结果：新容器内 `test -x /usr/local/bin/actrailctl`、`test -x /usr/local/bin/libactrail_tls_payload_probe_sync.so` 和 `test -x /root/.cargo/bin/xiaoo` 都成功。

迁移配置和密钥的操作：

```bash
docker exec "$AGENT_CONTAINER" mkdir -p /root/.config
docker exec "$EXISTING_AGENT_CONTAINER" tar -C /root/.config -cf - xiaoo | docker exec -i "$AGENT_CONTAINER" tar -C /root/.config -xf -
docker exec "$EXISTING_AGENT_CONTAINER" tar -C /root -cf - api_key.sh | docker exec -i "$AGENT_CONTAINER" tar -C /root -xf -
```

说明：只复制你确认需要的 xiaoO 配置和密钥路径，并把这一步当作敏感操作处理；如果你的密钥路径不是 `/root/api_key.sh`，替换成实际路径。

预期结果：新容器内 xiaoO 能读取配置和密钥；完成迁移后，回到主流程第 3 步重新检查容器，检查通过后继续执行主流程第 6 步。

旧容器的生命周期管理属于环境管理决策，不属于本文档的 AcTrail 部署步骤；本文档只描述保留旧容器并创建替代 workload 容器的做法。

### 2. 容器内没有 AcTrail release 组件

如果容器内没有 `/usr/local/bin/actrailctl` 或 `/usr/local/bin/libactrail_tls_payload_probe_sync.so`，优先在与目标容器 OS/GLIBC 兼容的独立构建环境中编译后复制进去。下面的“在目标容器内编译”只是一种兼容性兜底,不是要求生产 workload 容器安装 cargo/clang。

兜底操作：

```bash
export ACTRAIL_REPO_IN_CONTAINER=/path/to/AcTrail
docker exec -e ACTRAIL_REPO_IN_CONTAINER "$AGENT_CONTAINER" bash -lc '
  set -eu
  test -n "${ACTRAIL_REPO_IN_CONTAINER:-}"
  test "$ACTRAIL_REPO_IN_CONTAINER" != "/"
  test -f "$ACTRAIL_REPO_IN_CONTAINER/Cargo.toml"
  test -d "$ACTRAIL_REPO_IN_CONTAINER/crates"
  cd "$ACTRAIL_REPO_IN_CONTAINER"
  cargo fmt --all
  cargo build --release -p ctl -p tls_payload_probe_sync
  install -m 0755 target/release/actrailctl /usr/local/bin/actrailctl
  install -m 0755 target/release/libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
  rm -rf "${ACTRAIL_REPO_IN_CONTAINER:?}/target"
'
```

说明：这一步只编译容器侧最小运行组件，不在容器内编译 web，也不保留 AcTrail 仓库的 `target/`；`ACTRAIL_REPO_IN_CONTAINER` 必须改成容器内实际 AcTrail 仓库路径，命令会先检查 `Cargo.toml` 和 `crates/` 存在再清理 build 目录。

预期结果：容器内 `actrailctl` 和 TLS-sync `.so` 可执行，容器内没有遗留大体积 `target/`，后续可直接执行主流程第 6 步。

### 3. Agent 路径、密钥或代理不同

如果 xiaoO 不在 `/root/.cargo/bin/xiaoo`，或者密钥不是通过 `/root/api_key.sh` 注入，需要替换主流程第 6 步里的路径和 `source` 命令。

操作示例：

```bash
docker exec -e HTTP_PROXY="$AGENT_PROXY" -e HTTPS_PROXY="$AGENT_PROXY" -e NO_PROXY=localhost,127.0.0.1,::1 "$AGENT_CONTAINER" bash -lc 'source /actual/key/env/file && export PATH=/usr/local/bin:/actual/agent/bin:$PATH && actrailctl launch -- /actual/agent/bin/xiaoo --cli run -p "你好"'
```

说明：密钥文件缺失应该直接失败，不要用 `source xxx || true` 掩盖，否则会把模型请求失败误判成 AcTrail 观测失败。

预期结果：Agent 自己能正常访问模型服务，AcTrail trace 中能看到 Agent 进程、网络事件、TLS 明文载荷和 semantic action；路径替换完成后，按主流程第 6 步的方式执行替换后的 launch 命令。

### 4. 需要短生命周期 workload 容器

如果你不复用已有容器，而是希望每次 trace 都由 `docker run --rm` 创建一个短生命周期容器，则这个镜像必须已经包含 xiaoO、`actrailctl` 和 TLS-sync `.so`；模型密钥和用户配置不应该固化进镜像，推荐在运行时通过只读挂载或环境文件注入。

操作：

```bash
export AGENT_RUNTIME_IMAGE=actrail-oe2403-xiaoo-runtime:latest
export XIAOO_CONFIG_DIR=/path/to/xiaoo-config
export XIAOO_ENV_FILE=/path/to/xiaoo-env-file
export ACTRAIL_SECCOMP_PROFILE=/absolute/path/to/AcTrail/deploy/container-auto/seccomp/actrail-notify.json
test -f "$ACTRAIL_SECCOMP_PROFILE"
docker run --rm --name actrail-xiaoo \
  --user 0:0 \
  --security-opt "seccomp=$ACTRAIL_SECCOMP_PROFILE" \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$XIAOO_CONFIG_DIR:/root/.config/xiaoo:ro" \
  -v "$XIAOO_ENV_FILE:/run/secrets/xiaoo-env:ro" \
  -e HTTP_PROXY="$AGENT_PROXY" \
  -e HTTPS_PROXY="$AGENT_PROXY" \
  -e http_proxy="$AGENT_PROXY" \
  -e https_proxy="$AGENT_PROXY" \
  -e NO_PROXY=localhost,127.0.0.1,::1 \
  -e no_proxy=localhost,127.0.0.1,::1 \
  "$AGENT_RUNTIME_IMAGE" \
  bash -lc 'source /run/secrets/xiaoo-env && export PATH=/usr/local/bin:/root/.cargo/bin:$PATH && actrailctl launch -- /root/.cargo/bin/xiaoo --cli run -p "你好"'
```

说明：这是“已有容器模式”的替代方案，不是同一次 trace 还要额外启动的第二个 workload 容器；如果镜像不是按“其他分支情况 6”生成的 `actrail-oe2403-xiaoo-runtime:latest`，把 `AGENT_RUNTIME_IMAGE` 改成你实际已经准备好的镜像名；`XIAOO_CONFIG_DIR`、`XIAOO_ENV_FILE` 和 `ACTRAIL_SECCOMP_PROFILE` 必须指向主机上实际存在的路径。可信测试或兼容性排障可以将定制 profile 替换为 `--security-opt seccomp=unconfined`，但正常部署推荐保留 Docker 外层过滤。

预期结果：容器随 Agent 退出自动删除，主机 SQLite 中仍然保留 trace 数据。

### 5. 主机配置没有启用 TLS-sync

如果只看到进程和网络事件但没有 rustls payload 或 `llm.*` semantic action，先检查主机 `/etc/actrail/actraild.conf` 的 TLS plaintext 配置。

操作：

```bash
rg -n '^\[capture\]|^\[payload\.tls\]|^capabilities|^enabled|^capture_backend|^binary_path|^sync_event_socket_path' /etc/actrail/actraild.conf
```

说明：xiaoO/rustls 明文捕获依赖主机 daemon 的 TLS-sync 后端；semantic action 是 daemon 侧投影，但它依赖已经采集到的 LLM HTTP/TLS 应用载荷。

预期结果：`[capture] capabilities` 包含 `"tls-plaintext-payload"`；`[payload.tls]` 下包含 `enabled = true`、`capture_backend = "tls-sync"`、`binary_path = "disabled"` 和 `sync_event_socket_path = "/run/actrail/tls-sync.sock"`。修改配置后需要重启 `actraild`，然后回到主流程第 2 步重新执行 `doctor` 检查。

### 6. 必须打镜像保存环境

如果没有别的部署方式，只能把准备好的容器提交成镜像，提交前必须先清理构建产物和包缓存，并且不能破坏 xiaoO 和 AcTrail release 组件。不要把模型密钥或用户配置固化进镜像；如果候选容器 writable layer 里已经有密钥文件，先不要 commit 这个容器，改用运行时只读挂载或环境文件注入密钥。

操作示例：

```bash
export ACTRAIL_REPO_IN_CONTAINER=/path/to/AcTrail
docker exec -e ACTRAIL_REPO_IN_CONTAINER "$AGENT_CONTAINER" bash -lc '
  set -eu
  test -n "${ACTRAIL_REPO_IN_CONTAINER:-}"
  test "$ACTRAIL_REPO_IN_CONTAINER" != "/"
  test -f "$ACTRAIL_REPO_IN_CONTAINER/Cargo.toml"
  test -d "$ACTRAIL_REPO_IN_CONTAINER/crates"
  rm -rf "${ACTRAIL_REPO_IN_CONTAINER:?}/target"
  dnf clean all
'
docker exec "$AGENT_CONTAINER" test -x /usr/local/bin/actrailctl
docker exec "$AGENT_CONTAINER" test -x /usr/local/bin/libactrail_tls_payload_probe_sync.so
docker exec "$AGENT_CONTAINER" test -x /root/.cargo/bin/xiaoo
docker exec "$AGENT_CONTAINER" test ! -f /root/api_key.sh
docker exec "$AGENT_CONTAINER" test ! -e /root/.config/xiaoo
docker commit "$AGENT_CONTAINER" actrail-oe2403-xiaoo-runtime:latest
```

说明：打镜像是最后手段，不是默认部署路径；清理只应该发生在提交镜像前，一般测试任务不需要每次都清理；`test ! -f /root/api_key.sh` 和 `test ! -e /root/.config/xiaoo` 是示例密钥/配置路径检查，如果你的密钥或配置在其他路径，按实际路径补充同类检查。

预期结果：新镜像体积不包含 AcTrail `target/` 这类大目录，并且用“其他分支情况 4”的 `docker run --rm` 命令可以重新启动并完成 trace。

### 7. 常见失败解释

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| 容器内看不到 `/run/actrail/control.sock` | 主机 daemon 未启动，或容器创建时没有挂载 `/run/actrail`。 | 先启动主机 `actraild`，如果仍不存在就按“其他分支情况 1”保留旧容器并新建替代容器。 |
| `pidfd_getfd seccomp listener: Operation not permitted` | Docker 默认 seccomp profile 拦截 launch-time pidfd 路径，且 launch 使用了 `--seccomp-notify required`。 | 改用 `--seccomp-notify auto` 自动降级；若需要 seccomp-notify 轴，优先按“其他分支情况 1”使用定制 profile `actrail-notify.json`。`seccomp=unconfined` 仅用于可信测试、兼容性排障或明确接受更宽 syscall 面的环境。 |
| `launch command probe failed` 且 TLS payload 为空 | 动态 TLS probe plan 无法识别当前 Agent/运行时,或 Agent 命令指向了包装脚本而非真实可执行文件。 | 保持 TLS-sync 的 `[payload.tls] binary_path = "disabled"`；先用真实 Agent 可执行文件运行 finder/probe,确认该运行时受支持。参考 [container-agent-minimal](examples/container-agent-minimal/operator.conf) 和 [container-agent-restricted](examples/container-agent-restricted/README.md)。 |
| `actrailctl` 在主机能跑但在 openEuler 容器内不能跑 | 主机编译产物依赖了容器没有的较新 glibc。 | 按“其他分支情况 2”在目标容器 OS 内用 `cargo build --release` 编译。 |
| 没有 TLS payload rows | 没有通过 `actrailctl launch` 启动，TLS-sync `.so` 缺失，socket 未挂载，或 rustls probe plan 不匹配 Agent 二进制。 | 逐项检查主流程第 3、6、8 步；必要时先验证 `tls-probe-point-finder` 对 Agent 二进制的支持。 |
| 没有 `llm.*` semantic action | daemon 没收到可解析的 LLM HTTP/TLS 应用载荷。 | 先修复 TLS payload capture；semantic action 在主机 daemon 侧生成，不需要在容器内额外运行 semantic runtime。 |
| 容器镜像突然变得很大 | 把 `target/`、Cargo cache 或包管理器 cache 提交进镜像。 | 按“其他分支情况 6”在提交前清理，并优先避免非必要打镜像。 |
