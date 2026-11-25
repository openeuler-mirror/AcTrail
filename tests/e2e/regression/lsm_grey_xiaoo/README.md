# LSM grey authorization with real Xiaoo

## Test objective

Verify that grey access control applies external decisions to real Xiaoo operations, preserves command arguments and actor identity, and can reuse explicitly authorized decisions as kernel rules.

## Scenario design

This case launches the real Xiaoo executable with its Bash tool. A local streaming LLM provider requests a Python workload that accesses real files. The daemon uses a WASM control-decider to call an independent HTTP security service. The security service runs outside the controlled trace and writes its request log into the case directory.

The service alternates allow and deny for repeated `open` requests on the same file. Static allow and deny rules complete without service requests. Read rules receive allow during the sleepable open check; subsequent `file_permission` checks use the configured deny or allow fallback. Two more agent launches verify the service timeout and unavailable-service fallback.

A command phase launches the same curl executable against the same local target with explicit `-X GET` and `-X POST` arguments. The plugin queries the structured command context and sends the executable, complete argv, and actor identity to the service. The service allows GET and denies POST. The case checks the captured argv against the launched command, correlates the GET child PID, and verifies that only GET reaches the target. The command rule uses grey without an argv filter, so the external verdict determines execution.

An argv phase runs a native helper under Xiaoo. A parent thread continuously changes a shared argument buffer while children execute through execve and execveat. The service's argv is compared with the argv printed by each new executable image across 32 executions. Boundary cases allow 16 arguments and 255-byte strings, and reject 17 arguments or 256-byte strings before contacting the service, with command grey fallback explicitly set to allow.

### Management HTTP deadlines

Management commands have a 300 ms budget and a 5000 ms HTTP limit. The default
30 ms finish reserve leaves time for the component to return an HTTP error.
The service delays response headers and response bodies by two seconds in
separate calls. After each timeout, a fast command must succeed on the same
instance without reloading it. A second package sets `finish_reserve_ms=300`
and verifies that an insufficient budget returns an error without sending HTTP.
The case saves timings and responses in `management-deadlines.json` and checks
the actual service requests in `management-requests.json`.

### Reuse external decisions as kernel rules

The reuse phase alternates curl and cat executions in one Xiaoo trace. Both
executables initially match grey rules. The service denies curl and allows cat,
and explicitly returns `reuse: "executable"`. The plugin publishes an exact
executable rule through `command-policy.rules.apply` before replying to the
suspended exec. The rule priority is 1, above the initial grey priority of 0.
Load grants permit only curl deny and cat allow for their resolved paths.

The phase runs three curl/cat pairs by default. Set
`LSM_GREY_XIAOO_E2E_REUSE_ROUNDS` to a value of at least 2 to change the count.

| Stage | curl result | cat result | External requests | Audited rules |
| --- | --- | --- | --- | --- |
| First pair in the reuse trace | `EACCES` | Reads the expected content | One per executable | `curl-grey`, `cat-grey` |
| Remaining pairs in the same trace | `EACCES` | Reads the expected content | None | `reuse:<executable>` |
| All pairs in a new trace after the service stops | `EACCES` | Reads the expected content | None | `reuse:<executable>` |

The assertions check execution results, audit counts and exactly two external
requests across these stages.

These executable rules apply to all controlled traces and remain owned by the
plugin instance. Removing the rules or unloading that instance removes the
reused decisions. The service authorizes the whole executable regardless of its
arguments. The earlier GET/POST phase uses one-time decisions and runs before
the executable reuse phase.

## Test dependencies

| Dependency | Requirement | Purpose |
| --- | --- | --- |
| Python | Python 3.11 or newer | Run the test |
| Host | Linux, root, active BPF LSM and eBPF | Enforce real accesses |
| AcTrail | Release binaries and WASM build tools | Run the daemon and decision component |
| Xiaoo | Runnable CLI | Execute the real workload |
| Kernel module | Matching actrail_lsm_grey for the dynamic-decision case | Provide sleepable external decisions |
| C compiler | Available compiler | Build native argv workload |

## Test steps

1. Refresh defaults and start an isolated daemon, local provider and external security service.
2. Let Xiaoo execute actual file accesses, then compare syscall results with service requests and policy audit records.
3. Run GET and POST command decisions and native argv boundary scenarios.
4. Exercise management HTTP timeouts, then check that a fast request succeeds on the same instance.
5. Publish reusable executable decisions and repeat access in the same trace and a new trace after stopping the service.
6. Save workload results and request evidence, then stop the services.

| Check | Condition | Expected result |
| --- | --- | --- |
| File decisions | Repeated grey requests and static controls | Actual access follows the verdict, static rules need no service request |
| Command context | GET and POST against the same target | Full argv and actor match execution, only GET reaches the target |
| Argument integrity | Concurrent argument changes and boundary inputs | Service argv matches the executable image, oversized inputs fail before HTTP |
| Deadline handling | Headers or body exceed the command deadline | Request errors, the next fast command succeeds on the same instance |
| Decision reuse | Service authorizes executable reuse | Initial curl/cat pair makes two requests, later accesses use the published rules |

## Quick run


The test uses `actrailctl init -f` with an isolated patch and builds the WASM fixture in release mode. It selects grey `required` mode and requires a C compiler, an active BPF LSM and the matching `actrail_lsm_grey` module installed for `modprobe` or already loaded with its default argument limits for the running kernel.

Set `LSM_GREY_XIAOO_E2E_MODE=auto` to verify automatic module loading and dynamic decisions with the same assertions. The shared runner executes the release installer before the case, so an unloaded module can be built, installed and loaded through the normal startup path.

```sh
bash crates/access_control/kernel/build.sh
insmod crates/access_control/kernel/actrail_lsm_grey.ko
PATH=/root/.cargo/bin:$PATH \
LSM_GREY_XIAOO_E2E_BINARY=/root/.cargo/bin/xiaoo \
LSM_GREY_XIAOO_E2E_COMMAND_TIMEOUT_SECONDS=180 \
python3 tests/e2e/regression/lsm_grey_xiaoo/run_e2e.py --color never
```

The case copies available requests, workload results, launch logs, daemon logs, and failure details into `tmp/lsm-grey-evidence/run-*` under the repository before cleanup. Set `LSM_GREY_XIAOO_E2E_ARTIFACT_DIR` to select a persistent directory outside the runner's work directory. The shared runner removes its temporary workspace and runner log during default cleanup; the copied evidence remains. Each workload result records its PID and executable ancestry. The service records the decision ID, trace ID, PID, operation, target, ordinal, and verdict for every request.

The plugin calls the analysis endpoint through `http-host.request`. Its runtime
configuration supplies the URL, and `host.http.allowed_urls` authorizes it.
The test driver uses the other endpoints to control the service.

| Method and path | Request | Purpose and response |
| --- | --- | --- |
| `PUT /analyze` | Decision context as JSON and `X-Decision-Client: wasm-fixture` | Returns HTTP 202 with `verdict`, `reason`, and optional `reuse` |
| `POST /delay` | `{"seconds": 2}` | Delays analysis to test timeout handling, returns HTTP 200 |
| `POST /reuse` | `{"enabled": true}` | Enables executable-wide decisions, returns HTTP 200 |

The plugin checks the analysis response status and
`X-Decision-Source: mock-service` header before interpreting the JSON body.

## Startup mode checks without a module

On a machine where `actrail_lsm_grey` is neither installed nor loaded, build the release binaries and run:

```sh
cargo fmt --all
cargo build --release --bin actraild --bin actrailctl --bin actrailviewer
python3 tests/e2e/regression/test_all.py --case lsm_grey_xiaoo_modes
```

This entry point uses existing binaries and isolates every configuration and daemon socket. It starts real Xiaoo with refreshed defaults to verify `auto` fallback, `disabled` operation, and the non-LSM backend with `required` mode. LSM `required` must fail startup when the module is unavailable. It leaves logs and results in the printed evidence directory and stops its services. It does not install or load modules.

模块缺失场景通过 `python3 tests/e2e/regression/test_all.py --case lsm_grey_xiaoo_modes` 运行。该测例要求主机未安装和加载 grey 模块，使用统一入口提供的二进制目录和工作目录执行真实 XiaoO 启动模式矩阵。
