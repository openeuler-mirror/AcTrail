# V2 测试路径与职责约束

## 目标结构

```text
tests/v2/
├── README.md
├── common/                              # 跨 case 复用的测试框架能力
│   ├── actrail_runtime.py               # AcTrail 命令与 daemon 生命周期
│   ├── agent_selection.py               # Agent binary 探测与参数映射
│   ├── config.py                        # TestCaseInputs 和公共配置
│   ├── errors.py                        # 公共测试错误
│   ├── llm_trace_assertion.py           # 跨 case 的 LLM trace 断言
│   ├── output.py                        # runner 输出
│   ├── plugin_test_environment.py       # 插件测试环境生命周期
│   ├── plugin_web_api.py                # actrailweb 插件控制 API
│   ├── runner.py                        # 单 case/聚合执行与外层清理
│   ├── testing_context.py               # 一轮测试的共享上下文
│   ├── testing_env/
│   │   └── agent_availability.py        # Agent 外部条件预检
│   └── test_case/
│       ├── __init__.py                  # 仅 re-export 公共类型
│       ├── test_case.py                 # TestCase contract
│       ├── test_result.py               # 单项及组合结果
│       └── test_status.py               # PASSED/SKIPPED/FAILED
│
└── regression/                          # 所有 V2 Regression cases
    ├── README.md                        # 运行方式和 case 索引
    ├── test_all.py                      # 所有 V2 Regression 的聚合入口
    ├── <case_name>/
    │   ├── __init__.py
    │   ├── run_e2e.py
    │   ├── README.zh.md
    │   └── <case 专属实现>.py
    └── plugins/
        └── <plugin-id>/
            ├── __init__.py
            ├── run_e2e.py
            ├── README.zh.md
            └── <插件 case 专属实现>.py
```

`tests/v2/regression/test_all.py` 是唯一的聚合入口。领域子目录不得另建 runner、
公共参数或聚合入口。

## 路径选择

| 内容 | 路径 |
| --- | --- |
| 跨多个 case 复用的框架能力 | `tests/v2/common/` |
| 现有扁平 Regression case | `tests/v2/regression/<case_name>/` |
| 插件端到端 Regression case | `tests/v2/regression/plugins/<plugin-id>/` |
| 所有 V2 Regression 的注册与汇总 | `tests/v2/regression/test_all.py` |
| Regression 运行说明和索引 | `tests/v2/regression/README.md` |

`common/` 不按单个业务功能建立目录。只有在能力已经跨 case 复用，并且职责可以
独立说明时，才允许进入 `common/`。插件 Schema、某个插件的配置组合、特定导出
字段和业务断言必须留在对应插件 case 中。

## 文件职责

case 只强制存在以下三个文件：

```text
<case>/
├── __init__.py
├── run_e2e.py
└── README.zh.md
```

其余文件按职责拆分，不要求为了凑结构而创建：

| case 内文件 | 职责 |
| --- | --- |
| `config.py` | 接收 `TestCaseInputs`，补充 case 专属配置 |
| `case.py` | 外部预检、environment/task 编排和结果汇总 |
| `environment.py` | 该类被测环境的启动、恢复和停止 |
| `task.py` | 测试动作、轮询和断言；不负责最终清理 |

如果一个 case 足够简单，可以合并这些实现文件；但不得把 case 专属逻辑下沉到
`common/`，也不得让 `run_e2e.py` 重新实现 runner。

## 命名与导入

- `regression/<case_name>` 使用小写 snake_case，且目录名与
  `TestDefinition.name` 一致。
- `regression/plugins/<plugin-id>` 使用插件的 canonical id；因此
  `otel-jsonl` 这类带连字符的目录名是合法的。
- 带连字符的插件路径由聚合入口通过文件路径加载，不能假设它是普通 Python
  dotted package。
- 插件 case 的 `TestDefinition.name` 仍使用 snake_case，例如
  `plugin_otel_jsonl`，用于 CLI 筛选、workspace 和日志命名。
- `__init__.py` 只做 re-export，不读取环境变量、不注册 case，也不产生初始化
  副作用。
- case 不得 import 另一个 case 的内部实现；可复用能力应提取到 `common/`。

## 注册约束

新增 case 时必须同时：

1. 在 `tests/v2/regression/test_all.py` 注册；
2. 在 `tests/v2/regression/README.md` 增加文档索引；
3. 保留可独立运行的 `run_e2e.py`；
4. 让单 case 与 `test_all.py` 使用相同的公共 runner 参数和清理语义。

执行输入、workspace 所有权和清理职责见
[V2 测试执行输入与清理生命周期](execution-lifecycle.zh.md)。

## 设计文档路径

```text
docs/designs/tests/v2/
├── README.zh.md
├── v2-test-framework.zh.md
├── execution-lifecycle.zh.md
├── target-structure.zh.md
└── regression-authoring.zh.md
```
