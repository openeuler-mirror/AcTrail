# 停滞区间端到端验收

使用刷新后的隔离默认配置启动真实 OpenCode，执行模型请求、工具调用和权限等待，通过 Agent 插件暂停真实轮次产生停滞与恢复告警。

```bash
cargo fmt
cargo build --release --bin actraild --bin actrailctl --bin actrailweb
python3 tests/process/idle-detection/e2e.py \
  --output tmp/idle-acceptance \
  --opencode /path/to/opencode \
  --playwright /path/to/playwright/index.mjs \
  --chromium /path/to/chrome
```

输出目录必须尚不存在。脚本检查正常等待排除、开放和恢复后的停滞区间、终态历史、Web 重启后的稳定结果，以及每页一个候选时的分页、跨窗口筛选和空页继续。浏览器验证 waterfall 初始加载、显式加载停滞区间和替换式翻页。报告与截图写入输出目录，服务在结束时停止。
