# Probe dcode LLM

使用 `bpf-copy` TLS 配置，验证真实 deepagents-code（`dcode`）通过 AcTrail
启动后的 LLM 请求、响应采集与关联。

## 运行

在仓库根目录执行，保留当前用户的 PATH、HOME 和 provider 认证环境：

```bash
sudo -E env "PATH=$PATH" python3 tests/v2/regression/probe_dcode_llm/run_e2e.py
sudo -E env "PATH=$PATH" python3 tests/v2/regression/test_all.py \
  --no-profile --case probe_dcode_llm
```

测例已纳入默认回归集合。保留日志及工作目录时添加 `--no-cleanup`；
dcode thread 在每次调用结束时清理。

## 前提与配置

`dcode` 必须可从 PATH 找到，并已配置可访问的默认模型。
使用 `dcode --version` 和 `dcode --default-model` 查看实际版本与模型。
测例沿用 dcode 默认 provider/model，在 runner 分配的工作目录下为每次调用
创建独占临时目录。预检和观测运行均在退出后按该目录精确查询并删除 thread。
超时通过公共进程管理器终止进程组后执行清理。清理失败输出 warning，
保留该次调用的观测回归结果。

测例在 `<work-root>/probe_dcode_llm/` 生成 `actraild.conf` 和完整的
`actraild.patch.toml`，将 socket、PID、日志、数据库、导出和插件路径指向该工作目录。
配置 patch 合并本目录的 [`actraild.patch.toml`](actraild.patch.toml)，设置
`payload.tls.capture_backend = "bpf-copy"`。
初始化使用 `actrailctl --config <本地配置> init -f --patch <生成的patch>`；
daemon、ctl launch 和 viewer 均显式使用同一份本地配置。

`init` 默认使用 `complete` 模式，启动和运行期的直接探针发现沿用默认开启值。
runner 的 `--profile` 是机器环境配置，与 `init --mode profile` 含义不同。
TLS `bpf-copy` 每次 SSL 操作最多采集 65,535 字节，完整模式不消除这个后端限制。
LLM 连接的实际协议取决于 provider 配置；明文 HTTP 可由 socket 采集，因此
LLM 断言通过不单独证明 TLS uprobe 路径已被覆盖。

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
5. 安全停止 daemon 并等待数据收尾，验证 trace 处于干净终态。
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

目标 trace 应正常结束；LLM 请求与响应应有有效 call links，响应包含
测例输出的随机标记。自动测例执行完整关联断言。
