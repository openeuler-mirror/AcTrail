# AcTrail V2 测试设计文档

本目录定义 `tests/v2/` 的目标结构、测试结果语义、Regression case 编写规范
以及中文操作文档格式。它面向新增或评审 V2 测试的开发者，而不是某一次测试
运行的结果记录。

## 阅读顺序

1. [V2 测试框架规范](v2-test-framework.zh.md)
   - 定义 `SKIPPED`、`PASSED`、`FAILED` 的边界。
   - 定义 V2 测试的通用准则。

2. [V2 测试目标目录结构](target-structure.zh.md)
   - 定义 `tests/v2/` 的目标代码路径和注册点。

3. [V2 Regression 编写与文档规范](regression-authoring.zh.md)
   - 定义每个 case 的 `README.zh.md` 必须具有的章节。
   - 定义手动指令和预期结果的编写要求。

## 文档职责

| 文档 | 性质 | 更新时机 |
| --- | --- | --- |
| `v2-test-framework.zh.md` | 测试状态与通用准则 | 状态边界或公共准则变化时 |
| `target-structure.zh.md` | 目标代码路径 | 目录层次或注册点变化时 |
| `regression-authoring.zh.md` | README 编写规则 | 文档格式或手动验收要求变化时 |
| `README.zh.md` | 本目录索引 | 新增、删除或重命名本目录文档时 |

## 规范优先级

这些文档描述的是 V2 测试的目标规范。当前代码与规范不一致时，应明确判断：

1. 当前实现需要迁移；
2. 规范遗漏了有效场景，需要先修订；
3. 差异是有期限、有负责人的例外。

禁止为了让一次测试“变绿”而把 AcTrail 失败改成 `SKIPPED`，也禁止在没有可操作
复现步骤的情况下只保留宏观测试描述。
