# 插件故障排查

> 本文说明如何诊断插件未被发现、加载被拒绝、阻止 daemon 启动或运行后没有输出的问题。

## 插件未被发现

**检查：** 候选 package 必须是 `plugins.discovery.directory` 的直接子目录，并且只包含一个 `*.plugin.toml` manifest。

**处理：** 将 manifest、artifact、配置 schema 和 alert schema 放在同一 package 目录内；移除重复 manifest 和逃逸 package 目录的路径。

## 加载被拒绝

**检查：** 直接运行 load 命令并读取结构化错误，将 manifest 声明的 host capabilities 与传入的 `--grant` 比较。

**处理：** 提供全部必需 grant，且不提供未声明 grant。Environment、rule type 和 path 等参数化 scope 只开放插件所需范围；business config 必须通过 manifest 声明的 schema 校验。

## Startup plugin 阻止 daemon 启动

**检查：** 查看 `plugins.startup.failure_policy`，以及失败 entry 的 policy、manifest path、configuration path 和 grants。

**处理：** 修复安全边界所依赖的必需治理插件。只有明确可选的插件才使用 `continue`；部署必须依赖的控制保持 `fail-fast`。

## Instance active 但没有输出

**检查：** 确认 release binary 位于 `PATH`，并把 config path 与 instance name 替换为实际值：

```bash
sudo actraild --config /etc/actrail/actraild.conf plugin status \
  --instance my.instance
```

`active` 只表示 runtime instance 存在。继续检查 `last_error`、drop/retry counter、queue pressure 和 downstream reachability。

**处理：** 恢复 downstream service 或修正 exporter 配置。已有 instance active 时不重复 load；重复加载会掩盖原始故障并丢弃有界 runtime state。

## 修改文件后没有生效

**检查：** 确认 instance 来自 startup list 还是 runtime load。插件在 load 时读取 manifest、artifact 和 business config。

**处理：** Runtime instance 需要显式 unload 后重新 load；startup list 变更需要重启 daemon。同一个 instance 不同时交给 startup list 与 persisted runtime registration 管理。

正常生命周期操作见 [管理插件](../plugins/manage.md)，精确接口契约见 [插件 API](../../reference/plugin-api/README.md)。
