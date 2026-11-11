"""Synthetic stdio MCP server with externally controlled catalog notifications."""
import json
from pathlib import Path
import sys
import threading

control = Path(sys.argv[1])
lock = threading.Lock()
stopping = threading.Event()
initialized = threading.Event()
counts = {"lists": 0, "calls": 0}


def send(message):
    with lock:
        print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def state():
    try:
        return json.loads(control.read_text())
    except (OSError, ValueError):
        return {"tools": 0, "resources": 0, "prompts": 0}


def watch():
    previous = state()
    initialized.wait()
    while not stopping.wait(0.05):
        current = state()
        for family in ["tools", "resources", "prompts"]:
            if current[family] != previous[family]:
                send({"method": f"notifications/{family}/list_changed"})
        previous = current


threading.Thread(target=watch, daemon=True).start()
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    if method == "notifications/initialized":
        initialized.set()
    if "id" not in request:
        continue
    current = state()
    if method == "initialize":
        result = {"protocolVersion": "2025-11-25", "serverInfo": {"name": "catalog-fixture", "version": "1.0.0"}, "capabilities": {k: {"listChanged": True} for k in ["tools", "resources", "prompts"]}}
    elif method == "tools/list":
        result = {"tools": [{"name": f"read_{current['tools']}", "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "resources/list":
        result = {"resources": [{"uri": f"fixture://item/{current['resources']}", "name": "Fixture"}]}
    elif method == "resources/templates/list":
        result = {"resourceTemplates": []}
    elif method == "prompts/list":
        result = {"prompts": [{"name": f"summary_{current['prompts']}"}]}
    elif method == "tools/call":
        counts["calls"] += 1
        result = {"content": [{"type": "text", "text": "synthetic"}]}
    else:
        result = {}
    if method.endswith("/list"):
        counts["lists"] += 1
    control.with_suffix(".counts.json").write_text(json.dumps(counts))
    send({"id": request["id"], "result": result})
stopping.set()
