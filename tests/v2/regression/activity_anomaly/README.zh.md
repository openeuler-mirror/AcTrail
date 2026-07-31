# activity-anomaly 规则命中回归

该 v2 用例只验证 activity-anomaly 插件的核心契约：活动记录命中规则后生成告警。
测试构造确定命中的请求增长、响应增长和长命令记录，并检查每类告警的定义键、
去重键及非空 findings 负载。

该用例不启动真实 Agent、Docker、actraild 或网络服务，因此不依赖 xiaoO、Claude
或容器镜像。真实流量、多容器隔离、实时投递和重复插件去重属于独立的重型 E2E
覆盖范围，不在本回归用例中验证。

从仓库根目录通过聚合入口运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case plugin_activity_anomaly
```

也可以单独通过 v2 wrapper 运行：

```bash
sudo -E python3.11 \
  tests/v2/regression/activity_anomaly/run_e2e.py
```

也可以直接运行插件规则测试：

```bash
cargo test \
  --manifest-path examples/plugins/wit-component/activity-anomaly/Cargo.toml \
  matching_activity_builds_alert_drafts
```
