# 运维指南

> 本文提供 AcTrail 部署、日常运行与故障排查的运维文档入口。

## 部署

1. [选择部署模式](deployment/choose-a-mode.md)。
2. 按运行边界选择 [Linux 主机](deployment/host.md)、[Docker workload](deployment/container.md) 或 [执行隔离](deployment/execution-isolation.md)。
3. 启动前执行 [主机准备检查](troubleshooting/preflight.md)。

## 日常运行

- [生成和维护 daemon 配置](daemon/configure.md)
- [启动、停止和检查 daemon](daemon/start-stop.md)
- [查看和导出 trace](daemon/inspect-traces.md)
- [管理插件](plugins/manage.md)

## 集成

- [捕获 xiaoO rustls LLM 请求](integrations/xiaoo-rustls.md)

## 故障排查

- [采集结果缺失](troubleshooting/capture.md)
- [部署或启动失败](troubleshooting/deployment.md)
- [插件发现、加载或输出失败](troubleshooting/plugins.md)
