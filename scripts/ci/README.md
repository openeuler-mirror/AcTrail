# CI 检查

## 运行

在仓库根目录运行全部检查：

```bash
bash scripts/ci/run.sh
```

| 用途 | 命令 |
| --- | --- |
| 查看入口帮助 | `bash scripts/ci/run.sh --help` |
| 模块布局 | `bash scripts/ci/check-module-layout.sh` |
| 测试卫生 | `python3 scripts/ci/check-tests-hygiene.py` |
| Rust 格式 | `bash scripts/ci/check-fmt.sh` |
| 文档与代码布局 | `python3 scripts/ci/check-docs.py` |

入口按表中的检查顺序执行。一项检查发现问题后，继续执行其余检查，最后汇总结果。

汇总中的重跑命令应在仓库根目录执行。

## 环境要求

| 工具 | 要求 |
| --- | --- |
| Bash、Git | 可从 `PATH` 找到 |
| Python | 3.10 或更高版本 |
| Cargo、rustfmt | 已安装，能够运行 `cargo fmt` |

入口发现缺失命令时会报错。

## 检查内容

| 检查 | 检查范围 |
| --- | --- |
| 测试卫生 | 主代码中的 `#[test]`、`#[tokio::test]`、`#[async_std::test]` 会报错。允许独立 `tests/` 下的测试及其 fixture 源码 |
| Rust 格式 | 整个 workspace 的 rustfmt 排版 |
| 文档链接 | Markdown 本地链接和图片目标路径 |
| 代码布局路径 | `code-layout.md` 中标为 `[x]` 的文件和目录 |
| 代码布局规模 | 独立布局中实际存在的文件数 |

`src/tests/`、`src/tests.rs` 仍属于主代码。主代码可以通过 `#[cfg(test)]`、`#[path]` 引用外置测试。

Python 检查读取 Git 已跟踪文件和未被忽略的新文件，检查工作区当前内容。

### 文档与布局

| 项目 | 规则 |
| --- | --- |
| 本地链接 | 先相对文档目录查找，再相对仓库根目录查找 |
| 网络链接、标题锚点、代码示例 | 跳过 |
| 布局根路径 | 相对仓库根目录 |
| 已完成节点 `[x]` | 文件或目录必须存在且类型正确 |
| 文件下的类型和方法 | 作为定义说明，不计入文件数 |
| 文件数统计 | 按实际存在的文件路径去重 |
| 仓库总览 `docs/architecture/code-layout.md` | 豁免文件数阈值 |

## 配置布局文件数阈值

阈值必须为正整数，取值优先级如下：

| 优先级 | 配置来源 | 示例 |
| --- | --- | --- |
| 1，最高 | 文档检查的命令行参数 | `--min-layout-files 15` |
| 2 | 环境变量 | `ACTRAIL_CI_MIN_LAYOUT_FILES=15` |
| 3 | 内置默认值 | `10` |

调用示例：

```bash
# 统一入口使用环境变量
ACTRAIL_CI_MIN_LAYOUT_FILES=15 bash scripts/ci/run.sh

# 单独运行文档检查时使用命令行参数
python3 scripts/ci/check-docs.py --min-layout-files 15
```

## 日志位置与保留

每次运行使用独立目录，步骤编号从 1 开始：

```text
${TMPDIR:-/tmp}/actrail-ci-logs.XXXXXX/step-N.log
```

每份步骤日志包含该检查的 stdout 和 stderr。

| 情况 | 日志处理 |
| --- | --- |
| 某项检查通过 | 立即删除该项日志 |
| 所有检查通过，入口结束 | 删除日志目录 |
| 有检查失败，入口完成失败汇总 | 保留失败日志和目录 |
| 失败日志为空 | 保留文件，汇总中省略其路径 |
| 入口自身报错，未完成汇总便退出 | 删除日志目录 |
