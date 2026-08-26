# 启动、停止和检查 daemon

> 本文说明如何安全启动、检查、停止或重启已经配置的 AcTrail daemon。

以下命令使用系统默认配置 `/etc/actrail/actraild.conf`。独立实例应在每条命令中加入 `--config /path/to/operator.conf`。

命令假设 release binary 已安装到 `PATH`。从源码 checkout 运行时，可使用对应的 `./target/release/<binary>`。

## 后台运行

```bash
sudo actraild start
sudo actraild status
sudo actrailctl doctor
```

`start` 只有在 PID 文件和 control socket 都出现后才返回成功。它的等待预算还包含存储打开、collector preflight 和 startup plugin 加载。`doctor` 验证 control plane 和存储 readiness，不代表目标工作负载已经产生事件。

## 交给 service manager

Systemd 等 supervisor 应以前台模式运行 daemon：

```bash
sudo actraild run
```

Service manager 内不得再使用 `start` 创建第二层后台进程。

## 停止或重启

```bash
sudo actraild stop
sudo actraild restart
```

停止期间 daemon 会停止接收新控制工作，完成 terminal trace finalization，再 drain post-trace 和插件告警写入。超时会留下 degraded diagnostic 并返回错误；正常 `stop` 超时不会直接强杀仍在 drain 的 daemon。此时检查配置的 `log_path` 和受影响 trace 的 diagnostics，再决定是否进行主机级干预。

`actrailctl clean` 会删除配置声明的本地运行产物。它不适合日常停止，也不应对必须保留 SQLite、日志或 export 的生产配置执行。
