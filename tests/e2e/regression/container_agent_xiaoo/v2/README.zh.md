# Container Agent xiaoO V2

## 测试目标

验证两个 Docker 容器中并发运行的真实 xiaoO，其进程身份、trace 和观测内容保持正确归属。

## 场景设计

在独立 PID namespace 的两个长驻容器内先后启动真实 xiaoO，使用各自本地 Provider 执行不同任务。建立两个 Agent 同时活动的窗口，分别记录两个 Agent 的实际回复，以实际回复、文件内容和容器身份核对观测归属。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| AcTrail | 当前 release 产物 | 采集、存储和查询观测记录 |
| 主机 | Linux、root、可工作的 eBPF 环境 | 采集容器活动 |
| Docker | 可访问的 daemon 和测试镜像 | 运行隔离容器 |
| xiaoO | 可运行的真实 CLI | 执行两个任务 |
| 本地 Provider | 两个独立流式服务 | 提供可区分的任务响应 |

## 测试步骤摘要

1. 检查 release、Docker 和 xiaoO，准备镜像。
2. 刷新默认配置并启动隔离 daemon 与两个 Provider。
3. 启动两个容器并记录各自 container ID 和 PID namespace。
4. 先后启动两个 xiaoO，确认两个 trace 同时 Active。
5. 保存两个 Agent 的实际回复，以启动时记录的 trace 和容器身份查询观测，核对 eBPF、文件读写和 LLM 响应。
6. 清理本轮容器与运行目录。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 真实并发 | 两个 docker exec 尚未退出 | 两条 trace 存在同时 Active 的窗口 |
| 身份归属 | 已记录两个完整 container ID | 两条 trace 与容器一一对应，PID namespace 不同 |
| 观测完整性 | 两个任务完成 | 每条 trace 有 eBPF 进程和网络事件、正数文件读写字节及成功 LLM 响应 |
| 内容隔离 | 分别记录两个 Agent 的实际回复和文件操作结果 | 对应 trace 的成功响应包含该 Agent 的完整实际回复，文件记录与实际路径和内容一致，容器身份归属正确 |

## 快速运行

在仓库根目录执行：

```bash
sudo -E python3 tests/e2e/regression/container_agent_xiaoo/v2/run_e2e.py
```

该脚本模拟两个用户在两个独立 Docker 容器中并发运行真实 xiaoO。容器先以
`tail -f /dev/null` 长驻，xiaoO 再通过容器内的 `actrailctl launch` 启动。测试
验证容器归属、PID namespace、并发 trace、eBPF、文件访问和 LLM response 证据
不会跨容器串线。
每轮由公共 `PatchBuilder` 生成随机运行目录对应的 `runtime.patch.toml`，再执行 `actrailctl init -f --patch runtime.patch.toml --patch patch_config.toml`。后一个文件位于本测例目录，声明场景行为。文件观测显式开启
read/write 的 `observed` 和 `bytes`，关闭 `bulk_read`，以验证独立读写动作及字节数。
`sudo -E` 保留调用者已经导出的环境，runner、场景进程和 Docker 调用继续继承。

回测用例统一管理本轮 `/tmp/actrail-nested-observer.*` 目录，嵌套 Docker 数据
和内部 xiaoO 场景的 `scenario/` 目录都位于其中。默认在成功、失败及超时后清理，
删除失败会使回测报告清理错误。使用 `--no-cleanup` 可保留现场，日志中的
`nested_observer_runtime=` 给出路径。场景脚本的 `--runtime` 参数用于接收调用者
管理的目录，其删除职责由调用者承担。

嵌套 dockerd 运行在私有的 PID、网络和 mount namespace 中。场景先在该 mount
namespace 内用空 tmpfs 遮住宿主 `/run/containerd`，使 dockerd 启动自己的
containerd；否则它会接入宿主 containerd，容器进程落在嵌套 PID namespace 之外。

### 运行参数

本目录的 `constants.py` 使用 `ContainerAgentXiaooConstants` dataclass 定义测例默认值。
修改字段即可调整超时、轮询间隔、容器启动间隔、镜像和本地 LLM provider 参数。

- **进程退出**：`daemon_stop_timeout_seconds` 为 actraild 收到 SIGTERM 后的等待时间，默认 60 秒。超时后发送 SIGKILL，再按 `daemon_kill_timeout_seconds` 等待，默认 30 秒。
- **整轮执行**：`case_timeout_seconds` 默认 900 秒。整轮超时后的清理宽限由 `case_cleanup_grace_seconds` 控制，默认 120 秒。
- **参数覆盖**：runner 的 `CONTAINER_AGENT_XIAOO_E2E_TIMEOUT_SECONDS` 和 `CONTAINER_AGENT_XIAOO_E2E_CLEANUP_GRACE_SECONDS` 分别覆盖整轮超时和清理宽限。场景脚本已有的 CLI 参数覆盖对应默认值。
- **产品配置**：`patch_config.toml` 声明 AcTrail 的场景配置，通过 `actrailctl init --patch` 应用。

## 手动测试

以下命令均从仓库根目录执行。

### 步骤1：检查测试前提

#### 手动指令

```bash
BIN_DIR="${ACTRAIL_BIN_DIR:-target/release}"
XIAOO_BIN="${CONTAINER_AGENT_XIAOO_BINARY:-${XIAOO_BINARY:-$(command -v xiaoo)}}"
test "$(id -u)" -eq 0
test -x "$BIN_DIR/actraild"
test -x "$BIN_DIR/actrailctl"
test -x "$BIN_DIR/actrailviewer"
test -x "$XIAOO_BIN"
docker version --format '{{.Server.Version}}'
```

部分 Docker 版本（如 25.x）的 `docker info --format` 在 daemon 不可达时也返回 0，
因此测例用 `docker version`，并要求退出码为 0 且输出非空版本号。

#### 脚本行为与预期结果

AcTrail release 文件缺失判为 `FAILED`；Docker 或真实 xiaoO 不可用判为
`SKIPPED`。测试运行时只读取 `tests/e2e/regression/container_agent_xiaoo/v2/`
中的 Dockerfile、配置、workload 和 seccomp profile，不依赖旧测试目录。

### 步骤2：运行 V2 场景并保留诊断 runtime

#### 手动指令

```bash
sudo -E python3 \
  tests/e2e/regression/container_agent_xiaoo/v2/xiaoo_scenario.py \
  --bin-dir "$BIN_DIR" \
  --image "${CONTAINER_AGENT_XIAOO_IMAGE:-ubuntu:24.04}" \
  --xiaoo-bin "$XIAOO_BIN" \
  --keep-runtime
```

#### 脚本行为与预期结果

场景仅以缓存布局版本、基础镜像引用和 Dockerfile 内容计算稳定 CRC32 缓存键，
使用 `actrail/container-agent-xiaoo:runtime-v2-<cache-key>`。`actrailctl`、
xiaoO 和 workload 在运行时通过只读 bind mount 注入，因此它们
更新不会创建新镜像。已有同标签镜像时不执行 build；为上述手动命令追加
`--rebuild-image` 可强制重建。`run_e2e.py` 套件入口也可通过
`CONTAINER_AGENT_XIAOO_E2E_REBUILD_IMAGE=1` 请求重建。
stderr 最后输出保留的 `/tmp/actrail-multi-container-xiaoo.*` 路径。

- **配置生成**：先应用运行路径补丁，再应用本目录的 `patch_config.toml`。场景补丁声明文件读写统计等行为。
- **采集行为**：TLS、Socket 和启动权限使用产品默认值。验收检查完整 LLM 请求响应、文件读写及容器归属。
- **已有进程观察**：场景通过 `track-add` 验证已有进程的身份握手，因此关闭主动文件治理。
- **探针解析复用**：两个容器只读挂载同一个 xiaoO 文件。验收要求该可执行文件解析一次，随后命中缓存并完成两次探针挂载。

### 步骤3：确认长驻容器与 docker exec 拓扑

#### 手动指令

在场景运行期间另开终端执行：

```bash
docker ps --filter 'name=actrail-multi-xiaoo-' \
  --format '{{.ID}} {{.Names}} {{.Status}} {{.Command}}'
docker inspect --format '{{.Id}} pid={{.State.Pid}} status={{.State.Status}}' \
  $(docker ps -q --filter 'name=actrail-multi-xiaoo-')
```

#### 脚本行为与预期结果

应同时看到两个状态为 `running` 的容器，PID 1 执行
`tail -f /dev/null`。脚本记录两个宿主 PID 的 `/proc/<pid>/ns/pid`，两者必须
不同。xiaoO 退出只结束对应 `docker exec` 和 trace，不会提前销毁容器。

### 步骤4：确认两个真实 Agent 先后启动且同时 Active

#### 手动指令

```bash
RUNTIME_DIR="$(ls -dt /tmp/actrail-multi-container-xiaoo.* | head -n1)"
sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" '
SELECT trace_id, display_name, lifecycle_state, root_container_id, created_at
FROM traces
WHERE profile_name LIKE "multi-container-xiaoo%"
ORDER BY created_at;'
```

#### 脚本行为与预期结果

容器 A 先执行 release-summary 任务，默认 10 秒后容器 B 执行 security-review
任务。脚本要求两条 trace 曾同时为 `active`，对应的 `docker exec` 当时均未退出，
且创建时间满足配置的启动间隔。

### 步骤5：验证 trace 与容器归属

#### 手动指令

```bash
docker ps -a --filter 'name=actrail-multi-xiaoo-' \
  --no-trunc --format '{{.ID}} {{.Names}}'
sqlite3 "$RUNTIME_DIR/data/actrail.sqlite" '
SELECT trace_id, display_name, root_container_id
FROM traces
WHERE profile_name LIKE "multi-container-xiaoo%"
ORDER BY trace_id;'
```

#### 脚本行为与预期结果

数据库中的两个 `root_container_id` 必须与两个完整 Docker container ID 构成
一一对应；display name 必须分别为 `container-a-release-summary` 和
`container-b-security-review`，不能缺失或重复。

### 步骤6：验证 eBPF、文件和 LLM response

#### 手动指令

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

find "$RUNTIME_DIR/tasks/" -type f -print
```

#### 脚本行为与预期结果

分别保存两个真实 xiaoO 任务结束时的完整实际回复，并读取各自任务文件。
按启动时记录的 trace ID 和完整 container ID 查询观测。每条 trace 至少包含一个
eBPF process event、一个 eBPF network event、正数文件读写字节和一个成功的
`llm.response`。成功响应正文应包含对应 Agent 的完整实际回复，文件记录应与
该任务实际访问的路径和内容一致。以容器身份和文件归属判定跨容器串线。

### 步骤7：验证镜像复用

#### 手动指令

```bash
sudo -E python3 tests/e2e/regression/container_agent_xiaoo/v2/run_e2e.py
docker image ls actrail/container-agent-xiaoo --format '{{.Repository}}:{{.Tag}}'
sudo -E python3 tests/e2e/regression/container_agent_xiaoo/v2/run_e2e.py
docker image ls actrail/container-agent-xiaoo --format '{{.Repository}}:{{.Tag}}'
```

#### 脚本行为与预期结果

缓存布局版本、基础镜像引用和 Dockerfile 未变化时，两次镜像列表中的缓存标签
不变，第二次不执行 Docker build。
每轮测试只删除自己拥有的容器与 runtime；缓存镜像保留。
