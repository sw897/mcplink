"""Read-only Windows process observations exposed through a synthetic MCP server."""
import ctypes
import json
import os
import subprocess
import sys

from mcp_server import TOOLS, handle

# A descendant verifies that a daemon crash cleans the complete upstream job.
descendant = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(3600)"],
                              creationflags=subprocess.CREATE_NO_WINDOW)
TOOLS.append({"name": "windows_runtime", "description": "Read synthetic process metadata",
              "inputSchema": {"type": "object", "properties": {}},
              "annotations": {"readOnlyHint": True}})
try:
    for line in sys.stdin:
        request = json.loads(line)
        if "id" not in request:
            continue
        if request["method"] == "tools/call" and request.get("params", {}).get("name") == "windows_runtime":
            value = {"pid": os.getpid(), "descendant": descendant.pid,
                     "consoleWindow": ctypes.windll.kernel32.GetConsoleWindow()}
            result = {"content": [{"type": "text", "text": json.dumps(value)}], "isError": False}
        else:
            result = handle(request)
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
finally:
    descendant.terminate()
    descendant.wait(timeout=5)
