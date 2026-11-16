#!/usr/bin/env python3
"""Read a grey-protected file, then exec with arguments and attempt an outbound connection."""
import json
import os
import socket
import sys
import time
from pathlib import Path

class Workload:
    def run(self, work: Path, phase: str, port: int):
        result_path = work / f"results-{(work / 'phase').read_text().strip()}.json"
        # Each exec stage performs work after its execution event has been collected.
        time.sleep(2)
        if phase == "--read":
            result = {"errno": 0}
            try:
                result["content"] = (work / "protected/grey").read_text()
            except OSError as error:
                result["errno"] = error.errno
            result_path.write_text(json.dumps({"pid": os.getpid(), "results": [result]}))
            os.execv(sys.executable, [sys.executable, __file__, str(work), "--send", str(port)])
        result = {"errno": 0}
        try:
            with socket.socket() as connection:
                connection.settimeout(3)
                connection.connect(("127.0.0.1", port))
        except OSError as error:
            result["errno"] = error.errno
        document = json.loads(result_path.read_text())
        document["results"].append(result)
        result_path.write_text(json.dumps(document))
        print(json.dumps(document["results"]))

if __name__ == "__main__":
    Workload().run(Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]))
