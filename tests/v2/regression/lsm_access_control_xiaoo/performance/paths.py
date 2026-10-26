from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
OUTPUT_DIR = REPO / "local/lsm_access_control"
ENTRY = Path(__file__).with_name("run.py")
