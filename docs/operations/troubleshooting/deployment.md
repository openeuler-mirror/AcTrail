# 部署与启动故障

> 本文说明如何诊断 daemon 无法 ready、control socket 不可用或容器无法连接的问题。

## `start` 超时或 daemon 退出

以下命令假设 release binary 位于 `PATH`，并使用默认系统配置。

```bash
sudo actraild status
sudo tail -n 200 /var/log/actrail/actraild.log
```

独立实例应读取其 operator config 中的 `log_path`。常见原因是配置校验失败、写路径权限、SQLite 打开失败、eBPF preflight 或 startup plugin 加载失败。运维人员应先修复日志中的根因；只有确认正常初始化持续超过 `supervision.startup_wait_ms` 时才增加时间预算。

启动失败清理会停止所创建的 child 并清理 PID/control socket。若路径仍被占用，运维人员应确认没有另一个 daemon 使用同一配置；仍由存活进程绑定的 socket 不得直接删除。

## `doctor` 不能连接

运维人员应确认 daemon 与 ctl 使用同一配置，并检查以下项目：

```bash
sudo test -S /run/actrail/control.sock
sudo actraild status
```

自定义 `socket_path` 时必须在 daemon 与 ctl 两端使用同一 `--config` 或同一显式 socket override。检查 socket mode、属主和调用者组权限。

## 容器看不到 control socket

```bash
docker exec actrail-agent test -S /run/actrail/control.sock
docker inspect actrail-agent --format '{{json .Mounts}}'
```

容器创建时必须挂载主机 `/run/actrail`。`docker exec` 不能为已经创建的容器补挂载；保留原容器数据，创建一个带正确只读挂载的替代容器。

## `pidfd_getfd ... Operation not permitted`

Docker 默认 seccomp profile 可能阻止 launch-time listener 传递。先用：

```bash
docker exec actrail-agent actrailctl probe \
  --config /etc/actrail/actraild.conf \
  --host-ebpf auto \
  --seccomp-notify auto \
  --json
```

接受降级时保留 `auto`；必须使用 seccomp-notify 时，用仓库提供的 `deploy/container-auto/seccomp/actrail-notify.json` 重新创建容器并改为 `required`。`seccomp=unconfined` 会移除 Docker 外层 syscall 过滤，只能用于可信排障环境。
