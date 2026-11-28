# 回归测试：Web 渐进时间线

## 测试目标

验证 Web 能正确查询和展示真实采集的时间线、资源与正文，并在分页、筛选和交互过程中保持数据完整性。验证普通 HTTP 请求在默认网络视图中的可见性和详情，以及真实采集数据导入 Cluster 后的查询与身份一致性。

## 场景设计

运行真实 XiaoO 工具任务，使用刷新后的隔离默认配置采集数据。以同次采集的 SQLite 记录为依据核对 API，并通过浏览器操作时间线、资源区域和正文详情。用独立空资源配置验证无样本提示，用受限读取预算验证渐进加载。Cluster 场景使用真实采集目录完成正式打包与导入。

网络场景让真实 Agent 通过工具访问本地 HTTP 服务并将响应保存为文件。以服务实际收到的请求和保存的文件为依据，查找同一 trace 的 HTTP 记录，再在默认火焰图中定位并打开该请求。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| 主机环境 | Linux、root、可工作的采集环境 | 采集真实 Agent 活动 |
| AcTrail | 可运行的 release daemon、ctl、sb、web、viewer，Cluster 场景另需 cluster | 采集、查询与导入 |
| XiaoO 与模型服务 | CLI 可用、认证和模型配置有效 | 产生真实任务数据 |
| 浏览器环境 | Node.js、Playwright、Chromium | 操作页面并检查结果 |

## 测试步骤摘要

1. 刷新隔离默认配置，启动采集，运行真实 XiaoO 工具任务。
2. 结束采集，记录本轮 trace 和 SQLite 数据。
3. 启动 Web，对照存储事实核验分页、合并游标与进程筛选结果。
4. 用浏览器执行页面切换、时间轴缩放和平移、详情展开与资源折叠。
5. 用空资源和受限预算配置检查对应页面行为。
6. 在网络场景中核实真实 HTTP 请求和响应文件，打开默认火焰图。按请求发生时间缩小视窗，加载对应的详细记录，再检查请求标签、默认动作类别和详情。
7. 在 Cluster 场景中打包并导入真实数据，检查列表、UID 路由和重复导入结果。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 时间线数据 | 同次采集的存储记录与 API 结果 | action 集合一致，分页和合并游标无重复、遗漏 |
| 页面交互 | 首屏、页面切换、筛选与详情展开 | 查询符合操作范围，详情按需读取，无浏览器异常 |
| 资源展示 | 有样本与空资源两类配置 | 曲线实际绘制并与时间轴对齐，空数据有解释且无加载错误 |
| 正文 | 实际 manifest、blocks 与 Web 正文 | 内容、字节数及 UTF-8 前缀一致，按节点展开可用 |
| 首页查询 | 初始化、选定范围、编辑日期和刷新 | 查询由显式操作触发，已提交范围保持一致 |
| 普通网络请求 | 服务已收到真实 GET，响应文件已保存 | 默认视图包含对应 HTTP 消息，标签和详情对应实际方法与路径，消息显示切换保持动作类别与已加载数据 |
| Cluster | 打包导入、后续导入与重复导入 | UID 路由和分页正确，已有身份稳定 |

## 快速运行

### 运行指令

```bash
cargo fmt
cargo build --release --bin actraild --bin actrailctl --bin actrail-sb --bin actrailweb --bin actrailviewer
export WEB_TIMELINE_E2E_AGENT=/path/to/xiaoo
export WEB_TIMELINE_E2E_PLAYWRIGHT=/path/to/playwright/index.mjs
export WEB_TIMELINE_E2E_CHROMIUM=/path/to/chrome
sudo -E python3 tests/e2e/regression/test_all.py --case web_timeline --no-cleanup
```

### 场景与环境变量


三个场景通过 `TestCaseMeta` 自动登记，可分别执行。

```bash
sudo -E python3 tests/e2e/regression/test_all.py --case web_timeline --bin-dir target/release
sudo -E python3 tests/e2e/regression/test_all.py --case web_timeline_network --bin-dir target/release
sudo -E python3 tests/e2e/regression/test_all.py --case web_timeline_cluster --bin-dir target/release
```

这些场景需要 Node.js、Playwright、Chromium 和真实 XiaoO。场景配置使用 `WEB_TIMELINE_E2E_` 前缀的环境变量。

| 后缀 | 默认值或用途 |
| --- | --- |
| `AGENT` | PATH 中的 `xiaoo`，否则使用 `constants.py` 的 `agent_path` |
| `AGENT_TIMEOUT` | `constants.py` 的 `agent_timeout_seconds` |
| `PORT` | `constants.py` 的 `port` |
| `ENDPOINT` | `constants.py` 的 `endpoint` |
| `PLAYWRIGHT` | `constants.py` 的 `playwright_path` |
| `CHROMIUM` | `constants.py` 的 `chromium_path` |
| `SOURCE` | cluster 场景使用的真实采集目录。省略时在当前工作目录内执行真实 Agent 与浏览器采集验收，生成来源数据 |

统一入口管理工作目录和清理。使用 `--no-cleanup` 保留证据。这三个浏览器场景通过 `--case` 显式选择。

## 手动测试与详细验收


统一 runner 分配工作目录。测例使用独立配置、存储和监听端口，并在结束时停止测试服务。`WEB_TIMELINE_E2E_PORT` 默认为 `18479`。

运行参数由本目录 `constants.py` 中的 `WebTimelineConstants` dataclass 定义，包括超时、端口、浏览器尺寸、分页预算和内容大小。配置默认值读取这些字段。浏览器脚本通过同目录的 `constants.mjs` 读取该 dataclass，Python 与 Node.js 使用同一组默认值。

检查项包括真实 SQLite action 集合与 API 分页结果一致、多类型合并游标无重复或遗漏、进程筛选、跨窗口区间、非法页大小、空页继续、flamegraph 首屏请求、切换页面、waterfall 按需详情以及浏览器异常。小预算阶段单独启动 Web，限制每次读取一个候选。报告、请求耗时和截图保存在工作目录。

资源区域验收复用真实采集数据，检查 host/workload 曲线、来源标签、画布实际绘制、与火焰图时间轴的
横向对齐、缩放和平移后的查询覆盖，以及折叠/展开。随后用指向独立空资源路径的配置启动真实 Web，
要求 host/workload 都显示无样本解释且没有加载错误。该阶段不伪造资源 API 响应。

已有运行中的 Web 和真实 trace 时，可只执行资源浏览器检查：

```bash
node tests/e2e/regression/web_timeline/browser/resource-run.mjs \
  http://127.0.0.1:18479 1 /path/to/report \
  /path/to/playwright/index.mjs /path/to/chrome sampled
```

报告目录需预先存在；空资源配置的实例使用最后一个参数 `empty`，输出对应 JSON 报告和截图。

正文验收对照真实 SQLite manifest 与 blocks 检查内容一致性、精确字节数和 UTF-8 前缀边界；独立 Web 实例验证 7 字节叶子预览、浏览器按节点展开，以及内容块预算耗尽后其他接口仍可使用。

首页验收检查初始化不触发全局统计或清缓存，显式加载只提交一次选定范围查询，编辑日期保持已显示结果的范围，主动刷新使用已提交的范围。

Cluster 验收复用上述真实采集目录，通过正式 pack/import CLI 建立隔离中心，验证 UID 路由、列表分页、后续导入和重复导入后的身份稳定性：

```bash
cargo fmt -p web
cargo build --release --bin actrailcluster --bin actrailweb
sudo -E python3 tests/e2e/regression/test_all.py --case web_timeline_cluster --no-cleanup
```
