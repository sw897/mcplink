"""Synthetic MCP process tree for shutdown and cancellation acceptance."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from mcp_server import TOOLS, handle

parser = argparse.ArgumentParser()
parser.add_argument("--marker", type=Path, required=True)
parser.add_argument("--stubborn", action="store_true")
parser.add_argument("--hang-initialize", action="store_true")
args = parser.parse_args()
if args.stubborn and os.name != "nt":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGINT, signal.SIG_IGN)
child_code = "import time; time.sleep(3600)"
if args.stubborn and os.name != "nt":
    child_code = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(3600)"
child = subprocess.Popen([sys.executable, "-c", child_code],
    creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
observation = {"pid": os.getpid(), "descendant": child.pid, "calls": 0}
def persist():
    temporary = args.marker.with_suffix(".tmp")
    temporary.write_text(json.dumps(observation))
    temporary.replace(args.marker)
if os.name != "nt":
    observation["group"] = os.getpgrp()
persist()
TOOLS.append({"name": "process_info", "description": "Read synthetic process identifiers",
              "inputSchema": {"type": "object", "properties": {"delay": {"type": "number"}}},
              "annotations": {"readOnlyHint": True}})
try:
    for line in sys.stdin:
        request = json.loads(line)
        if "id" not in request:
            continue
        if args.hang_initialize and request["method"] == "initialize":
            while True:
                time.sleep(1)
        if request["method"] == "tools/call" and request.get("params", {}).get("name") == "process_info":
            observation["calls"] += 1
            persist()
            time.sleep(request.get("params", {}).get("arguments", {}).get("delay", 0))
            result = {"content": [{"type": "text", "text": json.dumps(observation)}], "isError": False}
        else:
            result = handle(request)
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
    if args.stubborn:
        while True:
            time.sleep(1)
finally:
    child.terminate()
    try:
        child.wait(timeout=2)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=2)
