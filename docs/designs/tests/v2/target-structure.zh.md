# V2 测试目标路径

## `tests/v2/`

```text
tests/v2/
├── README.md                         # V2 测试入口和通用规则
├── common/                           # 跨测试复用的公共能力
│   ├── test_case/                    # 测试用例的核心模型
│   │   ├── __init__.py               # 仅 re-export 公共类型
│   │   ├── test_case.py              # TestCase 定义和具体用例注册
│   │   ├── test_result.py            # 单项及组合测试结果
│   │   ├── test_status.py            # PASSED/SKIPPED/FAILED 等状态
│   │   └── common_test_config.py      # 所有 case 共用的配置
│   │
│   ├── test_runner/                  # 测试发现、执行和结果输出
│   │   ├── __init__.py               # 仅 re-export 公共类型和函数
│   │   ├── test_runner.py            # 单 case 和多 case 执行编排
│   │   ├── test_definition.py        # case 名称、描述和构建信息
│   │   └── test_output.py            # 终端与日志结果展示
│   │
│   ├── test_context/                 # 一轮测试共享的环境上下文
│   │   ├── __init__.py               # 仅 re-export 公共上下文
│   │   ├── testing_context.py        # 共享状态和环境检查入口
│   │   └── agent/                    # 外部 agent 测试条件
│   │       ├── __init__.py           # 仅 re-export agent 公共类型
│   │       ├── availability.py       # agent 登录、模型和网络预检
│   │       └── errors.py             # agent 外部条件错误
│   │
│   ├── actrail/                      # AcTrail 测试运行环境
│   │   ├── __init__.py               # 仅 re-export runtime 公共类型
│   │   ├── runtime.py                # binaries、命令执行和 daemon 生命周期
│   │   └── command_result.py         # AcTrail 命令执行结果
│   │
│   └── assertion/                    # 跨 case 复用的证据断言
│       ├── __init__.py               # 仅 re-export 公共断言
│       └── llm_trace.py              # LLM trace/action/link 验证
│
└── regression/                       # V2 Regression suite
    ├── README.md                      # 运行方式和 case 文档索引
    ├── test_all.py                    # case 注册、筛选和汇总入口
    └── <case_name>/                   # 一个独立 Regression case
        ├── __init__.py                # 仅 re-export case 公共入口
        ├── run_e2e.py                 # 单 case 可执行入口
        ├── README.zh.md                # Quick Run、摘要和手动步骤
        └── <该 case 所需的实现文件>.py  # case 自行决定的内部实现
```

## `common/` 收敛边界

| 路径 | 内容 |
| --- | --- |
| `test_case/` | case contract、result、status 和 case 公共配置 |
| `test_runner/` | case definition、执行编排和输出 |
| `test_context/` | 一轮测试共享的上下文和外部条件检查 |
| `test_context/agent/` | agent 可用性及其专属错误 |
| `actrail/` | AcTrail binaries、命令结果和 runtime lifecycle |
| `assertion/` | 可跨 case 复用的 AcTrail 证据断言 |

`common/` 根目录不再放业务实现文件，只保留这些 package。每个 package 通过
自己的 `__init__.py` 暴露稳定入口，内部文件不作为跨 package import 路径。

所有 `__init__.py` 只做 re-export，不放实现、注册副作用、环境读取或初始化
逻辑。

新的公共能力先判断属于哪个边界；不能明确归属时，不得继续向 `common/` 根目录
增加文件。

## Regression case

单个 case 只强制存在：

```text
<case_name>/
├── __init__.py
├── run_e2e.py
└── README.zh.md
```

其余文件按该 case 自身复杂度组织，不强制使用 `config.py`、`task.py` 或
`case.py`。

## 命名与注册

- case 目录使用小写 snake_case。
- case 名称与 `TestDefinition` 中注册的名称一致。
- 新 case 加入 `tests/v2/regression/test_all.py`。
- 新 case 加入 `tests/v2/regression/README.md` 的文档索引。
- case 不直接 import 另一个 case 的内部实现。

## 设计文档

```text
docs/designs/tests/v2/
├── README.zh.md
├── v2-test-framework.zh.md
├── target-structure.zh.md
└── regression-authoring.zh.md
```
