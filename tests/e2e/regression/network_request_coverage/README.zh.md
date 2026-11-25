# 回归测试：请求观测覆盖

## 测试目标

验证 AcTrail 能记录失败的模型请求和普通网络请求，并区分请求的性质与结果（失败的模型请求保留 LLM 错误记录，普通 HTTP 请求只产生网络记录）。

## 场景设计

在开启 LLM 采集的默认配置下，分别产生两类真实请求。

- **失败的模型请求。** 通过 AcTrail 启动 XiaoO，使用模型服务会拒绝的模型名称发起请求。核对该次请求的网络错误响应和 LLM 错误记录，以及二者所属的同一次调用。
- **普通 HTTP 请求。** 测例启动公共 [local_maas_server](../../common/test_suites/local_maas_server/README.md)，通过 AcTrail 启动 curl 访问其帮助接口（`GET /help`，命令行 HTTP 客户端，可在 shell 中执行）。核对请求路径、响应状态和请求响应配对，并确认该次访问没有被识别成LLM调用。

每次启动记录独立的 trace ID，以此查阅对应观测记录。公共服务使用独立进程和本地端口，通过公共进程工具管理启动与关闭（`SubprocessRunner`）。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测例及本地 HTTP 服务 |
| 主机环境 | Linux、root 权限及可工作的 eBPF 环境 | 采集进程和网络活动 |
| AcTrail | release 产物 `actraild`、`actrailctl`、`actrailviewer` | 采集、存储及查询 |
| XiaoO 与模型服务 | CLI 可用、认证有效，服务拒绝指定的模型名称 | 产生真实失败模型请求 |
| curl | 可执行的命令行客户端 | 产生普通 HTTP 请求 |

## 测试步骤摘要

1. **检查运行条件。** 检查主机权限及 AcTrail 产物。
2. **准备观测环境。** 在独立工作目录刷新默认配置（`actrailctl init`），启动 AcTrail，保持 LLM 采集开启。
3. **产生失败模型请求。** 先通过真实模型请求检查 XiaoO 可用性，再通过 AcTrail 启动 XiaoO，指定服务会拒绝的模型名称，记录 trace ID 和 Agent 执行结果。
4. **产生普通 HTTP 请求。** 检查 curl 可用性，启动公共服务并等待就绪，通过 AcTrail 启动 curl 访问 `/help`，记录服务响应状态及该次启动的 trace ID，随后关闭公共服务。
5. **完成数据采集。** 等待客户端退出，停止 AcTrail，等待数据落盘。
6. **核对观测记录。** 分别按两个 trace ID 查询请求、响应及其关联。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 失败请求可见 | 模型服务拒绝真实 Agent 请求 | 模型请求及错误响应均被记录，响应状态为失败，保留实际 HTTP 错误状态码 |
| 失败调用归属 | 已记录失败模型请求与响应 | 请求和响应各归属一个调用（`llm.call`），两者关联的网络消息也组成同一请求响应对；trace、进程和连接身份一致 |
| 普通请求可见 | curl 完成本地 HTTP 访问 | 网络记录包含实际 GET 路径和服务返回的状态码，请求与响应互相对应，属于该次启动 |
| 普通请求分类 | LLM 采集开启 | 普通访问的 trace 下不存在 LLM 记录（`llm.*`） |

缺少运行依赖时，对应场景报告 `SKIPPED` 并说明原因。服务接受指定的错误模型、未形成失败请求时，失败模型场景报告 `SKIPPED`。已建立场景后，记录缺失、状态错误或关联错误报告失败。两个场景分别记录结果。

## 快速运行

配置可正常访问模型服务的 XiaoO，在仓库根目录执行：

```bash
sudo -E python3 tests/e2e/regression/test_all.py --case network_request_coverage
```

模型服务需要拒绝错误模型名称。测例的错误模型、Agent 路径及执行超时可通过下列环境变量设置。

| 环境变量 | 默认行为 | 用途 |
| --- | --- | --- |
| `NETWORK_REQUEST_COVERAGE_E2E_ERROR_MODEL` | `actrail-e2e-nonexistent-model` | 指定模型服务会拒绝的模型名称 |
| `NETWORK_REQUEST_COVERAGE_E2E_XIAOO_BINARY` | 使用 `XIAOO_E2E_BINARY`，未设置时自动发现 XiaoO | 指定 Agent 可执行文件 |
| `NETWORK_REQUEST_COVERAGE_E2E_TIMEOUT_SECONDS` | 180 秒 | 命令及 Agent 任务的等待时限 |
| `NETWORK_REQUEST_COVERAGE_E2E_MAX_TURNS` | 1 | 失败模型任务的轮次上限 |
| `NETWORK_REQUEST_COVERAGE_E2E_PROMPT` | 简短描述文本文件的开放式问题 | 触发模型请求，不要求固定回复 |
| `NETWORK_REQUEST_COVERAGE_E2E_HTTP_PORT` | 自动选择可用本地端口 | 指定普通 HTTP 服务的端口 |
| `ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS` | 60 秒 | 真实 Agent 可用性检查的等待时限 |
