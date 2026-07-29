# Local MaaS Server 目标架构

## 文件路径

```text
tests/v2/common/test-suites/local-maas-server/
├── README.md
├── server.py
├── config.py
│
├── docs/
│   ├── architecture.md
│   ├── https.md
│   ├── scenario-format.md
│   ├── protocol-adapters.md
│   └── response-scheduling.md
│
├── scenario/
│   ├── __init__.py
│   ├── model.py
│   ├── runtime.py
│   │
│   └── scenario_generator/
│       ├── __init__.py
│       ├── config.py
│       ├── interface.py
│       ├── factory.py
│       ├── loader.py
│       │
│       ├── impl/
│       │   ├── __init__.py
│       │   ├── response.py
│       │   ├── sequential.py
│       │   ├── loop.py
│       │   └── random.py
│       │
│       └── templates/
│           ├── finite-sequence.json
│           ├── finite-middle-loop.json
│           ├── alternating-message-loop.json
│           ├── bash-tool-roundtrip.json
│           ├── bash-home-loop.json
│           └── random-message.json
│
├── protocol/
│   ├── __init__.py
│   ├── config.py
│   ├── interface.py
│   ├── registry.py
│   ├── openai.py
│   └── anthropic.py
│
├── schedule/
│   ├── __init__.py
│   ├── config.py
│   └── controller.py
│
├── server_core/
│   ├── __init__.py
│   ├── config.py
│   ├── application.py
│   ├── api_endpoints.py
│   │
│   └── connection/
│       ├── __init__.py
│       ├── interface.py
│       ├── factory.py
│       ├── manager.py
│       │
│       ├── http/
│       │   ├── __init__.py
│       │   ├── config.py
│       │   ├── server.py
│       │   └── handler.py
│       │
│       └── https/
│           ├── __init__.py
│           ├── config.py
│           ├── server.py
│           └── certificate.py
│
└── utils/
    ├── __init__.py
    ├── json.py
    └── logging.py
```

## 启动调用关系

```text
server.py
    ↓
LocalMaaSConfig
    ├── ScenarioGeneratorConfig
    ├── ProtocolConfig
    ├── ScheduleConfig
    └── ServerCoreConfig
            ↓
ScenarioLoader
    ↓
ScenarioGeneratorFactory
    ↓
ScenarioRuntime
    ↓
LocalMaaSApplication
    ↓
ConnectionFactory
    ├── HTTPConnectionServer
    └── HTTPSConnectionServer
            ↓
ConnectionManager
```

```text
ProtocolRegistry
    ↓
ApiEndpoints
    ↓
LocalMaaSApplication
    ↓ inject
ConnectionServer
    ↓ store
ConnectionDescription
    ↓ aggregate
ConnectionManager.describe
    ↓
StartupLogger
```

## Connection 调用关系

```text
ConnectionManager
    └── ConnectionServer[]
            ↑
            ├── HTTPConnectionServer
            │       └── HTTPRequestHandler
            │
            └── HTTPSConnectionServer
                    ├── HTTPRequestHandler
                    └── EphemeralCertificate
                            ↓
                         SSLContext
```

```text
HTTPSConnectionServer
    └── HTTPRequestHandler

HTTPConnectionServer
HTTPSConnectionServer
    └── ConnectionServer
```

## 请求调用关系

```text
HTTPConnectionServer
HTTPSConnectionServer
    ↓
HTTPRequestHandler
    ↓
ApiEndpoints
    ↓
ProtocolAdapter.decode_request
    ↓
ScenarioRuntime.reserve
    ↓
ProtocolAdapter.encode_response
    ↓
ScheduleController.apply
    ↓
HTTPRequestHandler
    ↓
direct response / SSE frames
```

## TLS 调用关系

```text
HTTPSConfig
    ↓
EphemeralCertificate
    ├── CA certificate
    ├── server certificate
    └── server private key
            ↓
         SSLContext
            ↓
HTTPSConnectionServer
```
