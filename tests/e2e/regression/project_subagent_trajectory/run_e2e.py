#!/usr/bin/env python3
"""Run this regression scenario independently."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import run_one
from tests.e2e.regression.project_subagent_trajectory.case import ProjectSubagentTrajectoryCase

if __name__ == "__main__":
    raise SystemExit(run_one(ProjectSubagentTrajectoryCase, REPO))
