# Container Agent xiaoO V2

# Quick Run

在仓库根目录执行：

```bash
sudo -E python3 tests/v2/regression/container_agent_xiaoo/v2/run_e2e.py
```

该脚本模拟两个用户在两个独立 Docker 容器中并发运行真实 xiaoO。容器先以
`tail -f /dev/null` 长驻，xiaoO 再通过容器内的 `actrailctl launch` 启动。测试
验证容器归属、PID namespace、并发 trace、eBPF、文件访问和 LLM response 证据
不会跨容器串线。
`sudo -E` 保留调用者已经导出的环境，runner、场景进程和 Docker 调用继续继承。

# 步骤摘要

1. 检查 AcTrail release 二进制、Docker daemon 和真实 xiaoO。
2. 按稳定运行时缓存键复用镜像；宿主二进制和脚本通过只读挂载注入。
3. 启动两个本地流式 LLM provider 和隔离的 AcTrail daemon。
4. 拉起两个 `tail -f /dev/null` 长驻容器并确认 PID namespace 不同。
5. 先后通过 `docker exec` 启动两个真实 xiaoO，制造可验证的 Active 重叠窗口。
6. 验证 trace 与 Docker container ID 一一对应。
7. 验证每条 trace 的 eBPF、文件和 LLM response marker 均完整且互不串线。
8. 删除本轮容器与 runtime，保留缓存镜像。

# 手动测试

以下命令均从仓库根目录执行。

## 步骤1：检查测试前提

### 手动指令

```bash
BIN_DIR="${ACTRAIL_BIN_DIR:-target/release}"
XIAOO_BIN="${CONTAINER_AGENT_XIAOO_BINARY:-${XIAOO_BINARY:-$(command -v xiaoo)}}"
test "$(id -u)" -eq 0
test -x "$BIN_DIR/actraild"
test -x "$BIN_DIR/actrailctl"
test -x "$BIN_DIR/actrailviewer"
test -x "$BIN_DIR/libactrail_tls_payload_probe_sync.so"
test -x "$XIAOO_BIN"
docker info --format '{{.ServerVersion}}'
```

### 脚本行为与预期结果

AcTrail release 文件缺失判为 `FAILED`；Docker 或真实 xiaoO 不可用判为
`SKIPPED`。测试运行时只读取 `tests/v2/regression/container_agent_xiaoo/v2/`
中的 Dockerfile、配置、workload 和 seccomp profile，不依赖旧测试目录。

## 步骤2：运行 V2 场景并保留诊断 runtime

### 手动指令

```bash
sudo -E python3 \
  tests/v2/regression/container_agent_xiaoo/v2/xiaoo_scenario.py \
  --bin-dir "$BIN_DIR" \
  --image "${CONTAINER_AGENT_XIAOO_IMAGE:-ubuntu:24.04}" \
  --xiaoo-bin "$XIAOO_BIN" \
  --keep-runtime
```

### 脚本行为与预期结果

场景仅以缓存布局版本、基础镜像引用和 Dockerfile 内容计算稳定 CRC32 缓存键，
使用 `actrail/container-agent-xiaoo:runtime-v2-<cache-key>`。`actrailctl`、TLS
probe、xiaoO 和 workload 不烘入镜像，运行时通过只读 bind mount 注入，因此它们
更新不会创建新镜像。已有同标签镜像时不执行 build；为上述手动命令追加
`--rebuild-image` 可强制重建。`run_e2e.py` 套件入口也可通过
`CONTAINER_AGENT_XIAOO_E2E_REBUILD_IMAGE=1` 请求重建。
stderr 最后输出保留的 `/tmp/actrail-multi-container-xiaoo.*` 路径。

## 步骤3：确认长驻容器与 docker exec 拓扑

### 手动指令

在场景运行期间另开终端执行：

```bash
docker ps --filter 'name=actrail-multi-xiaoo-' \
  --format '{{.ID}} {{.Names}} {{.Status}} {{.Command}}'
docker inspect --format '{{.Id}} pid={{.State.Pid}} status={{.State.Status}}' \
  $(docker ps -q --filter 'name=actrail-multi-xiaoo-')
```

### 脚本行为与预期结果

应同时看到两个状态为 `running` 的容器，PID 1 执行
`tail -f /dev/null`。脚本记录两个宿主 PID 的 `/proc/<pid>/ns/pid`，两者必须
不同。xiaoO 退出只结束对应 `docker exec` 和 trace，不会提前销毁容器。

## 步骤4：确认两个真实 Agent 先后启动且同时 Active

### 手动指令

```bash
RUNTIME_DIR="$(ls -dt /tmp/actrail-multi-container-xiaoo.* | head -n1)"
sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" '
SELECT trace_id, display_name, lifecycle_state, root_container_id, created_at
FROM traces
WHERE profile_name LIKE "multi-container-xiaoo%"
ORDER BY created_at;'
```

### 脚本行为与预期结果

容器 A 先执行 release-summary 任务，默认 10 秒后容器 B 执行 security-review
任务。脚本要求两条 trace 曾同时为 `active`，对应的 `docker exec` 当时均未退出，
且创建时间满足配置的启动间隔。

## 步骤5：验证 trace 与容器归属

### 手动指令

```bash
docker ps -a --filter 'name=actrail-multi-xiaoo-' \
  --no-trunc --format '{{.ID}} {{.Names}}'
sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" '
SELECT trace_id, display_name, root_container_id
FROM traces
WHERE profile_name LIKE "multi-container-xiaoo%"
ORDER BY trace_id;'
```

### 脚本行为与预期结果

数据库中的两个 `root_container_id` 必须与两个完整 Docker container ID 构成
一一对应；display name 必须分别为 `container-a-release-summary` 和
`container-b-security-review`，不能缺失或重复。

## 步骤6：验证 eBPF、文件和 LLM response

### 手动指令

```bash
sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" '
SELECT trace_id, collector, kind, COUNT(*)
FROM events
GROUP BY trace_id, collector, kind
ORDER BY trace_id, collector, kind;'

for TRACE_ID in $(sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" \
  'SELECT trace_id FROM traces ORDER BY trace_id;'); do
  "$BIN_DIR/actrailviewer" --config "$RUNTIME_DIR/operator.conf" \
    --output-format json actions --trace-id "$TRACE_ID" |
    jq '[.actions[] | select(.kind == "llm.response") |
      {status, text: .attributes["llm.response.content_text"]}]'
done

rg 'ACTRAIL_TASK_[AB]_' "$RUNTIME_DIR/tasks/"
```

### 脚本行为与预期结果

每条 trace 至少包含一个 eBPF process event、一个 eBPF network event、正数文件
读写字节和一个成功的 `llm.response`。A 的 response 只能包含
`ACTRAIL_TASK_A_RELEASE_SUMMARY_COMPLETE`，B 只能包含
`ACTRAIL_TASK_B_SECURITY_REVIEW_COMPLETE`；任一 request、response 或文件 marker
出现在另一条 trace 都判失败。

## 步骤7：验证镜像复用

### 手动指令

```bash
sudo -E python3 tests/v2/regression/container_agent_xiaoo/v2/run_e2e.py
docker image ls actrail/container-agent-xiaoo --format '{{.Repository}}:{{.Tag}}'
sudo -E python3 tests/v2/regression/container_agent_xiaoo/v2/run_e2e.py
docker image ls actrail/container-agent-xiaoo --format '{{.Repository}}:{{.Tag}}'
```

### 脚本行为与预期结果

缓存布局版本、基础镜像引用和 Dockerfile 未变化时，两次镜像列表中的缓存标签
不变，第二次不执行 Docker build。
每轮测试只删除自己拥有的容器与 runtime；缓存镜像保留。
