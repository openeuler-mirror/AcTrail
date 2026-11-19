"""Internal workload process launched through the real agent tool."""
import argparse
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[5]))
from tests.e2e.regression.lsm_access_control_xiaoo.performance.worker import Worker

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", type=str, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--rounds", type=int, required=True)
    parser.add_argument("--warmup", type=int, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--xiaoo", type=Path, required=True)
    parser.add_argument("--daemon-pid", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--allow-port", type=int, required=True)
    parser.add_argument("--deny-port", type=int, required=True)
    Worker(parser.parse_args()).run()
