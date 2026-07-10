# Qoder LLM Codec 插件示例

这个示例演示 Qoder LLM codec 插件如何把 Qoder CLI 的网络数据转换为 AcTrail 的 LLM 语义动作：

- 未加载插件时，真实 `qodercli` 调用不会产生 Qoder codec 提供的 `llm.request` 和 `llm.response`；
- 加载插件后，同样的调用会产生 `llm.request` 和 `llm.response`；
- 可以通过 `actrailviewer` 查看模型、提示词和响应内容。

下面的流程假设你在仓库根目录执行命令。

## 前置条件

需要先有 release 二进制：

```bash
cargo build --release
```

默认配置和 TLS plaintext capture 通常需要 root 或等价权限。下面命令统一用 `sudo` 演示。

确认 `qodercli` 已安装，并且 sudo 运行环境可以使用当前 Qoder 登录状态：

```bash
sudo env "PATH=$PATH" qodercli -p "请只输出 ACTRAIL_QODER_CODEC_PRECHECK"
```

预期输出包含：

```text
ACTRAIL_QODER_CODEC_PRECHECK
```

这个示例目录已经包含编译好的插件产物：

```text
examples/plugins/wasm-legacy/llm-codec-qoder/qoder_llm_codec.wasm
```

首次试用不需要重新编译 `.wasm`。

## 1. 确认默认配置

本示例使用 AcTrail 默认配置 `/etc/actrail/actraild.conf`。未指定 `--config` 时，`actraild`、`actrailctl` 和 `actrailviewer` 都会读取这个文件。

初始化或校验默认配置：

```bash
sudo target/release/actraild init
```

预期输出显示配置已初始化或校验成功，且 `/etc/actrail/actraild.conf` 存在。默认配置使用以下运行路径：

```text
/run/actrail/control.sock
/run/actrail/tls-sync.sock
/var/lib/actrail/actrail.sqlite
/var/log/actrail/actraild.log
```

只有改用其他配置文件时，才需要在后续命令中添加 `--config <path>`。

## 2. 启动 daemon

```bash
sudo target/release/actraild start
```

预期输出类似：

```text
actraild started pid=<PID> socket=/run/actrail/control.sock
```

检查 daemon 状态和 control socket：

```bash
sudo target/release/actraild status
sudo target/release/actrailctl doctor
```

预期现象：`status` 显示 daemon 正在运行，`doctor` 成功返回。

确认当前没有加载本示例插件：

```bash
sudo target/release/actraild plugin list
```

预期现象：输出中没有 `qoder.llm-codec`。如果已经存在，先执行：

```bash
sudo target/release/actraild plugin unload --instance qoder.llm-codec
```

预期输出：

```text
unloaded instance=qoder.llm-codec
```

## 3. 未加载插件时运行 QoderCLI

通过 `actrailctl launch` 运行真实 Qoder CLI：

```bash
sudo env "PATH=$PATH" target/release/actrailctl launch -- \
  qodercli -p "请只输出 ACTRAIL_QODER_CODEC_BASELINE"
```

预期输出包含：

```text
trace trace-<N> entered Active
ACTRAIL_QODER_CODEC_BASELINE
```

取最新 trace id：

```bash
BASELINE_TRACE_ID=$(sudo target/release/actrailviewer traces --tail 1 | awk 'NR==3 {print $1}')
printf 'BASELINE_TRACE_ID=%s\n' "$BASELINE_TRACE_ID"
```

预期输出类似：

```text
BASELINE_TRACE_ID=trace-12
```

查看这次调用的语义动作：

```bash
sudo target/release/actrailviewer actions \
  --trace-id "$BASELINE_TRACE_ID" --head 120
```

预期现象：可以看到 `process.exec`、`command.invocation` 等动作，但没有 Qoder codec 产生的 `llm.request` 或 `llm.response`。

## 4. 加载插件

```bash
sudo target/release/actraild plugin load \
  --manifest examples/plugins/wasm-legacy/llm-codec-qoder/plugin.toml \
  --instance qoder.llm-codec
```

预期输出：

```text
loaded instance=qoder.llm-codec
warnings=none
```

查看插件状态：

```bash
sudo target/release/actraild plugin status --instance qoder.llm-codec
```

预期输出包含：

```text
purpose=llm-codec
runtime=wasm
state=active
last_error=none
warnings=none
```

## 5. 加载插件后再次运行 QoderCLI

```bash
sudo env "PATH=$PATH" target/release/actrailctl launch -- \
  qodercli -p "请只输出 ACTRAIL_QODER_CODEC_DOC_OK"
```

预期输出包含：

```text
trace trace-<N> entered Active
ACTRAIL_QODER_CODEC_DOC_OK
```

取最新 trace id：

```bash
QODER_TRACE_ID=$(sudo target/release/actrailviewer traces --tail 1 | awk 'NR==3 {print $1}')
printf 'QODER_TRACE_ID=%s\n' "$QODER_TRACE_ID"
```

预期输出类似：

```text
QODER_TRACE_ID=trace-13
```

## 6. 查看 LLM 语义动作

```bash
sudo target/release/actrailviewer actions \
  --trace-id "$QODER_TRACE_ID" --tail 200
```

预期能看到：

```text
llm.request
llm.response
```

`llm.request` 表示 Qoder CLI 的 request body 已被插件转换为标准 LLM 请求；`llm.response` 表示 Qoder CLI 的 Server-Sent Events（SSE）数据已被插件解码并组装为模型响应。

使用 JSON 输出检查模型和测试标记：

```bash
sudo target/release/actrailviewer --output-format json actions \
  --trace-id "$QODER_TRACE_ID" \
  | grep -oE '"kind": "llm\.(request|response)"|"llm\.request\.model": "auto"|ACTRAIL_QODER_CODEC_DOC_OK' \
  | sort -u
```

预期输出至少包含：

```text
"kind": "llm.request"
"kind": "llm.response"
"llm.request.model": "auto"
ACTRAIL_QODER_CODEC_DOC_OK
```

这说明插件已经把 Qoder 请求和响应转换为 AcTrail 可以查询的 LLM 语义动作。

## 7. 清理

卸载插件：

```bash
sudo target/release/actraild plugin unload --instance qoder.llm-codec
```

预期输出：

```text
unloaded instance=qoder.llm-codec
```

确认插件已卸载：

```bash
sudo target/release/actraild plugin list
```

预期现象：输出中不再出现 `qoder.llm-codec`。

如果当前主机不再需要继续采集，停止 daemon：

```bash
sudo target/release/actraild stop
```

预期输出类似：

```text
actraild stopped pid=<PID>
```

`/etc/actrail/actraild.conf` 和 `/var/lib/actrail/actrail.sqlite` 会保留，供后续继续使用或审计本次 trace。

## 常见问题

### 加载插件时提示 unknown variant "llm-codec"

这个错误说明正在运行的 daemon 不支持当前 manifest 中的 `llm-codec` role。先停止旧 daemon，再用当前仓库构建的 release 二进制启动：

```bash
sudo target/release/actraild stop
sudo target/release/actraild start
```

然后重新执行插件加载命令。

### 只有 llm.response，没有 llm.request

确认默认配置中的 `[payload.tls]` 已启用，`capture_backend` 为 `"tls-sync"`，并检查本次 trace 的 request payload 是否完整。被截断的 request payload 不能生成完整的 `llm.request`。

## 从源码重新构建插件

只有修改了本目录的 Rust 源码时才需要重新构建：

```bash
rustup target add wasm32-unknown-unknown
cargo build \
  --manifest-path examples/plugins/wasm-legacy/llm-codec-qoder/Cargo.toml \
  --target wasm32-unknown-unknown \
  --release
cp \
  examples/plugins/wasm-legacy/llm-codec-qoder/target/wasm32-unknown-unknown/release/qoder_llm_codec.wasm \
  examples/plugins/wasm-legacy/llm-codec-qoder/qoder_llm_codec.wasm
```

预期现象：`qoder_llm_codec.wasm` 被新的 release 插件产物覆盖。

## 文件说明

| 文件 | 说明 |
| --- | --- |
| `plugin.toml` | 插件 manifest。 |
| `qoder_llm_codec.wasm` | 可直接加载的 WASM core module 插件产物。 |
| `Cargo.toml` / `Cargo.lock` | 重新构建插件时使用的 Rust crate 元数据。 |
| `src/lib.rs` | Qoder request body 和 SSE event data 的解码实现。 |
| `README.zh.md` | 本说明文档。 |

相关接口说明见 [WASM Core Module ABI](../../../../docs/plugins/abi/wasm-core-module.zh.md) 和 [LLM Codec ABI](../../../../docs/plugins/abi/llm-codec.zh.md)。
