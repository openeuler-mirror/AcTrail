# AcTrail Regression Runner

This directory provides a tester-facing regression entrypoint and writes human-readable and machine-readable reports.

```bash
uv venv --python /usr/bin/python3
source .venv/bin/activate
uv pip install -r tests/regression/requirements.txt
python3 tests/regression/test_all.py
```

Useful variants:

```bash
python3 tests/regression/test_all.py --list
python3 tests/regression/test_all.py --case e2e-xiaoo
python3 tests/regression/test_all.py --suite full --strict
python3 tests/regression/test_all.py --output-dir /tmp/actrail-regression
```

Cases are discovered from `tests/regression/cases/*/test.py`. Missing optional agents or credentials are reported according to each case's policy; actual failures in selected runnable cases are reported as `FAIL`.

The xiaoO case checks the configured CLI outside AcTrail before starting capture. It then requires a complete `llm.request` and `llm.response` semantic exchange and exports both action kinds as OTEL spans.

If an explicit binary override is invalid, the selected case fails fast. Without an override, unavailable optional dependencies are reported as `SKIP` with the scanned candidates.

Status markers:

```text
[√] pass
[x] fail
[-] skip
[!] warn
```
