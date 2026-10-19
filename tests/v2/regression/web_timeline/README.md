# 渐进时间线端到端验收

运行真实 Xiaoo 工具任务，使用当前 binary 刷新的隔离默认配置采集数据，并在浏览器验证时间线查询。需要可工作的 Xiaoo 模型配置、采集权限、Node.js、Playwright 和 Chromium。

```bash
cargo fmt
cargo build --release --bin actraild --bin actrailctl --bin actrail-sb --bin actrailweb --bin actrailviewer
sudo -E python3 tests/v2/regression/web_timeline/run.py \
  --work tmp/web-timeline-acceptance \
  --agent /path/to/xiaoo \
  --playwright /path/to/playwright/index.mjs \
  --chromium /path/to/chrome
```

`--work` 必须指向尚不存在的目录。脚本使用独立配置、存储和监听端口，并在结束时停止测试服务。`--port` 默认为 `18479`，可覆盖。

检查项包括真实 SQLite action 集合与 API 分页结果一致、多类型合并游标无重复或遗漏、进程筛选、跨窗口区间、非法页大小、空页继续、flamegraph 首屏请求、切换页面、waterfall 按需详情以及浏览器异常。小预算阶段单独启动 Web，限制每次读取一个候选。报告、请求耗时和截图保存在工作目录。

正文验收对照真实 SQLite manifest 与 blocks 检查内容一致性、精确字节数和 UTF-8 前缀边界；独立 Web 实例验证 7 字节叶子预览、浏览器按节点展开，以及内容块预算耗尽后其他接口仍可使用。

首页验收检查初始化不触发全局统计或清缓存，显式加载只提交一次选定范围查询，编辑日期保持已显示结果的范围，主动刷新使用已提交的范围。

Cluster 验收复用上述真实采集目录，通过正式 pack/import CLI 建立隔离中心，验证 UID 路由、列表分页、后续导入和重复导入后的身份稳定性：

```bash
cargo fmt -p web
cargo build --release --bin actrailcluster --bin actrailweb
python3 tests/v2/regression/web_timeline/cluster.py \
  --source tmp/web-timeline-acceptance --work tmp/web-cluster-acceptance \
  --playwright /path/to/playwright/index.mjs --chromium /path/to/chrome
```
