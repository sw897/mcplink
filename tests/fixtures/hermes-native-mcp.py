"""Drive the unmodified Hermes MCP discovery/registry without starting a model session."""
import contextlib
import json
from pathlib import Path
import sys

# The caller supplies a pinned vendor checkout and an isolated HERMES_HOME.
sys.path.insert(0, str(Path(sys.argv[1]).resolve()))
with contextlib.redirect_stdout(sys.stderr):
    from tools.mcp_tool_discovery import discover_mcp_tools
    from tools.mcp_tool_lifecycle import shutdown_mcp_servers
    from tools.registry import registry

try:
    for line in sys.stdin:
        request = json.loads(line)
        try:
            with contextlib.redirect_stdout(sys.stderr):
                method, params = request["method"], request.get("params", {})
                if method == "initialize":
                    discover_mcp_tools()
                    result = {"native": "Hermes MCP discovery and tool registry"}
                elif method == "tools/list":
                    result = registry.get_tool_names_for_toolset("mcp-mcplink")
                elif method == "tools/call":
                    result = registry.dispatch(params["name"], params["arguments"])
                    if isinstance(result, str):
                        try:
                            result = json.loads(result)
                        except json.JSONDecodeError:
                            result = {"content": [{"type": "text", "text": result}]}
                else:
                    raise ValueError("Unexpected fixture command")
            response = {"id": request["id"], "result": result}
        except Exception as error:
            response = {"id": request["id"], "error": {"message": str(error)}}
        print(json.dumps(response), flush=True)
finally:
    with contextlib.redirect_stdout(sys.stderr):
        shutdown_mcp_servers()
