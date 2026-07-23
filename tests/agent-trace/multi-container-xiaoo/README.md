# Multi-container xiaoO E2E

This case runs one host `actraild` and two real xiaoO processes in two
independent Docker containers. Both containers mount the same control and
TLS-sync Unix sockets. The test keeps both traces active concurrently and
requires host eBPF and the socket-payload seccomp fallback for both. The
repository's Docker seccomp profile keeps Docker's outer filtering while
allowing the `pidfd_getfd` operation required by that fallback.

The containers have deliberately different display names and tasks:

- `container-a-release-summary` reads release notes, writes a summary staging
  file, and sends a release-summary prompt to xiaoO;
- `container-b-security-review` reads a security policy, writes a review
  staging file, and sends a security-review prompt to xiaoO.

Container B starts 10 seconds after container A by default. Container A stays
active long enough to preserve a concurrent observation window.

The provider side is the repository's local OpenAI-compatible streaming
server. It keeps the test deterministic and credential-free while xiaoO still
performs its normal provider request, streaming response parsing, and agent
lifecycle.

The acceptance checks require:

- two different Docker container IDs and PID namespaces;
- a period where both traces are simultaneously `Active`;
- eBPF process and network events for each trace;
- positive `file.read` and `file.write` actions for each task's own paths;
- successful seccomp listener registration and vectored socket-payload
  capture for each trace;
- successful `llm.call`, `llm.request`, and `llm.response` actions for each
  xiaoO process;
- request and response markers assigned only to their owning trace.

Run after a release build:

```bash
sudo python3 tests/agent-trace/multi-container-xiaoo/run_e2e.py
```

Override the runtime image or xiaoO path when needed:

```bash
sudo python3 tests/agent-trace/multi-container-xiaoo/run_e2e.py \
  --image openeuler/openeuler:24.03-lts-sp3 \
  --xiaoo-bin /root/.cargo/bin/xiaoo \
  --container-start-stagger-seconds 10
```

On legacy Docker/runc combinations that cannot load the repository's current
profile, a trusted compatibility host can run the same acceptance case with
Docker's outer filter explicitly disabled:

```bash
sudo python3 tests/agent-trace/multi-container-xiaoo/run_e2e.py \
  --seccomp-profile unconfined
```

`unconfined` is only a compatibility-test option. Normal deployments should
keep the supplied `actrail-notify.json` profile.
