# 查看和导出 trace

> 本文说明如何查看 trace 中的进程、原始证据、语义动作和诊断，并按需导出结果。

以下示例使用默认配置；独立实例需为每条命令加 `--config /path/to/operator.conf`。

命令假设 release binary 已安装到 `PATH`。从源码 checkout 运行时，可使用对应的 `./target/release/<binary>`。

## 找到 trace

```bash
sudo actrailctl list-traces
sudo actrailviewer traces
```

`actrailctl` 从运行中的 daemon 获取控制面状态；`actrailviewer` 从存储读取记录。后续查询使用输出中的数字 trace ID。

## 由概览下钻

```bash
sudo actrailviewer summary --trace-id <TRACE_ID>
sudo actrailviewer processes --trace-id <TRACE_ID>
sudo actrailviewer actions --trace-id <TRACE_ID>
sudo actrailviewer diagnostics --trace-id <TRACE_ID>
```

语义 action 不完整时，继续检查底层事实：

```bash
sudo actrailviewer events --trace-id <TRACE_ID> --head 80
sudo actrailviewer network --trace-id <TRACE_ID> --head 40
sudo actrailviewer payloads --trace-id <TRACE_ID> --head 40
```

以下命令读取一个 retained payload：

```bash
sudo actrailviewer payload \
  --trace-id <TRACE_ID> \
  --segment-id <SEGMENT_ID> \
  --format text
```

非 UTF-8 内容使用 `--format hex`。

## 导出

```bash
sudo actrailviewer export-json \
  --trace-id <TRACE_ID> \
  --output /var/lib/actrail/export/trace-<TRACE_ID>.json

sudo actrailviewer export-otel \
  --trace-id <TRACE_ID> \
  --output /var/lib/actrail/export/trace-<TRACE_ID>.otlp.json
```

导出前应检查 `[export.snapshot]` 和 semantic body export 设置。JSON、OTLP 和实时插件的内容边界彼此独立；单个出口没有 body 不能证明其他出口也没有 body。
