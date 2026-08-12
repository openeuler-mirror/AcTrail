# HTTPS

## 启动

HTTPS 默认使用 OpenSSL 生成临时 CA 和服务端证书，并在临时端口 best-effort 启动：

```bash
python3 tests/v2/common/test_suites/local_maas_server/server.py \
  replay \
  --scenario alternating-message-loop \
  --http-bind-port 20000
```

默认 HTTPS 失败不会影响 HTTP；启动信息会显示失败 warning。传入任一 HTTPS/TLS 参数表示本次测试明确要求 HTTPS，此时失败会终止启动。

固定 HTTPS 端口：

```bash
python3 tests/v2/common/test_suites/local_maas_server/server.py \
  replay \
  --scenario alternating-message-loop \
  --http-bind-port 20000 \
  --https-bind-port 30000
```

端口设为 `0` 时由操作系统分配。`--https-bind-host` 省略时沿用 `--http-bind-host`。使用 `--disable-https` 可只启动 HTTP。

可配置入口：

```text
--https-bind-host
--https-bind-port
--tls-work-dir
--tls-openssl-binary
--tls-certificate-validity-days
--disable-https
```

## 启动信息

启动信息按 listener 展示实际 host、port、origin、服务类型和 REST API。HTTPS listener 同时展示临时 CA bundle 路径。

每个 connection server 在绑定端口后保存自己的结构化 description。协议端点由 protocol registry 和 API endpoint 表生成并注入 description，connection 不自行拼接协议路径。

## 客户端信任

服务优先读取启动环境中已有的 `SSL_CERT_FILE`；未设置时使用系统默认 CA bundle。它把该 bundle 与 Local MaaS CA 合并为临时 `combined-ca.pem`。

启动信息的最后一行会重点显示 `Please run with the Local MaaS CA: SSL_CERT_FILE=... <command>`。替换 `<command>` 后，环境变量只作用于该命令及其子进程，不会覆盖当前 shell。测试客户端不需要服务端私钥：

```bash
SSL_CERT_FILE=/tmp/local-maas-tls-.../combined-ca.pem \
OPENAI_BASE_URL=https://127.0.0.1:30000/v1 \
agent-command
```

环境变量只设置在 Agent 子进程中，不安装系统 CA。不同客户端使用其真实支持的 CA 配置入口。

证书 SAN 包含 `localhost`、`127.0.0.1` 和 `::1`。使用其他具体 bind host 时，该 host 也会写入 SAN。

## 清理

服务正常退出或收到 SIGTERM 时停止 HTTP/HTTPS listener，并删除其临时证书目录。`--tls-work-dir` 指定的是临时证书目录的父目录，服务只删除自己创建的子目录。
