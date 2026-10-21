# Probe dcode LLM

使用刷新后的默认采集配置，验证真实 deepagents-code（`dcode`）通过 AcTrail
启动后的 LLM 请求、响应采集与关联。

## 运行

在仓库根目录执行，保留当前用户的 PATH、HOME 和 provider 认证环境：

```bash
sudo -E env "PATH=$PATH" python3 tests/v2/regression/probe_dcode_llm/run_e2e.py
sudo -E env "PATH=$PATH" python3 tests/v2/regression/test_all.py \
  --no-profile --case probe_dcode_llm
```

- **回归入口**：测例已纳入默认回归集合。
- **保留产物**：保留日志及工作目录时添加 `--no-cleanup`。
- **会话清理**：dcode thread 在每次调用结束时清理。

## 前提与配置

### 运行环境与清理

- **程序与模型**：`dcode` 必须可从 PATH 找到，并已配置可访问的默认模型。使用 `dcode --version` 和 `dcode --default-model` 查看实际版本与模型。
- **调用隔离**：测例沿用 dcode 默认 provider/model，在 runner 分配的工作目录下为每次调用创建独占临时目录。
- **会话清理**：预检和观测运行均在退出后按该目录精确查询并删除 thread。
- **超时处理**：超时通过公共进程管理器终止进程组后执行清理。
- **清理异常**：清理失败输出 warning，保留该次调用的观测回归结果。

### 配置生成与使用

- **配置位置**：公共配置构建器在 `<work-root>/probe_dcode_llm/` 生成隔离路径补丁。daemon 数据、日志和 socket 使用该测例工作目录。
- **采集后端**：TLS 和 socket 继承 `init` 生成的默认后端，当前均为 `bpf-copy`，seccomp-notify 关闭。
- **配置初始化**：初始化使用 `actrailctl --config <本地配置> init -f --patch <隔离路径补丁>`，生成测例使用的 `actraild.conf`。
- **配置使用**：daemon、ctl launch 和 viewer 均显式使用同一份本地配置。

### 采集模式与覆盖

- **初始化模式**：`init` 默认使用 `complete` 模式，启动和运行期的直接探针发现沿用默认开启值。
- **运行器参数**：runner 的 `--profile` 是机器环境配置，与 `init --mode profile` 含义不同。
- **实际协议**：LLM 连接的实际协议取决于 provider 配置。明文 HTTP 由 socket 采集。
- **TLS 覆盖**：确认 TLS 用户态探针（uprobe）覆盖时，还需检查 TLS 来源的采集证据。

### 超时配置

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `DCODE_E2E_COMMAND_TIMEOUT_SECONDS` | `30` | AcTrail 生命周期、查询及 thread 清理命令超时，同时用作进程组终止宽限时间 |
| `DCODE_E2E_LAUNCH_TIMEOUT_SECONDS` | `180` | 每次真实 dcode 预检、观测运行超时，同时传给 dcode `--timeout` |

通用 runner 参数及 release 安装行为见[回归说明](../README.md)。

## 验收流程

1. 从 PATH 解析实际 dcode，使用随机标记运行一次外部可用性预检。
   缺少可执行文件、预检超时、异常退出或未返回标记时标记为 `SKIPPED`。
2. 在测例工作目录生成配置 patch，执行带本地 `--config` 的
   `init -f --patch`、stop、clean、start，刷新默认配置并启动采集。
3. 生成新的随机标记，通过 `actrailctl --config <本地配置> launch -- dcode -n <prompt> -q
   --no-mcp --timeout <seconds>` 请求原样回答，不使用工具。
4. 检查退出码、回答标记和唯一 trace id。
5. 停止 daemon 并等待数据收尾，确认停止命令成功，trace 的 `state` 为
   `Completed` 或 `Exited`，且 `health` 为 `Clean`。
6. 使用公共 `LLMTraceAssertion` 验证 LLM call/request/response 终态、
   有效关联和请求内容证据，确认关联响应成功且完整、包含本次标记。

预检和观测调用各自在 `finally` 中执行 thread 清理，覆盖正常返回、异常退出
和超时路径。清理使用 `dcode threads list --cwd <独占目录> --json` 查询，
再调用 `dcode threads delete <thread-id> --json` 删除。

AcTrail 启动错误、观测运行失败、缺失请求或响应以及错误关联均判为 `FAILED`。

## 手动复现

先运行测例生成并保留本地配置：

```bash
sudo -E env "PATH=$PATH" python3 tests/v2/regression/probe_dcode_llm/run_e2e.py \
  --work-root "$PWD/tmp/dcode-probe/work" \
  --log-dir "$PWD/tmp/dcode-probe/logs" --no-cleanup
```

以下命令查看该次运行的采集结果：

```bash
DCODE_CASE_DIR="$PWD/tmp/dcode-probe/work/probe_dcode_llm"
DCODE_CONFIG="$DCODE_CASE_DIR/actraild.conf"
sudo -E target/release/actrailviewer --config "$DCODE_CONFIG" --output-format json traces
```

使用 launch 输出的数字 trace id 查看采集内容：

```bash
sudo -E target/release/actrailviewer --config "$DCODE_CONFIG" \
  --output-format json actions --trace-id <trace-id>
```

目标 trace 应正常结束。LLM 请求与响应应有有效 call links，响应包含
测例输出的随机标记。自动测例执行完整关联断言。
