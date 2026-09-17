#!/usr/bin/env python3
"""Exercise the bundled stdio handshake without observing or controlling a desktop."""
import argparse
import json
import os
from pathlib import Path
import subprocess

TOOLS = {"list_apps", "get_app_state", "click", "perform_secondary_action", "scroll", "drag",
         "type_text", "press_key", "set_value", "screen_capture"}


def check(directory):
    manifest = json.loads((directory / "manifest.json").read_text())
    env = dict(os.environ, OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY="1")
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "BioRouter-build-check", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    result = subprocess.run([str(directory / manifest["executable"]), "mcp"],
                            input="".join(json.dumps(m) + "\n" for m in messages), text=True,
                            capture_output=True, timeout=30, env=env, check=True)
    replies = {item["id"]: item for item in (json.loads(line) for line in result.stdout.splitlines()) if "id" in item}
    if not replies.get(1, {}).get("result", {}).get("serverInfo"):
        raise ValueError("Native helper failed MCP initialization")
    initialized = replies[1]["result"]
    if initialized.get("protocolVersion") != "2025-03-26":
        raise ValueError("Native helper MCP protocol version drift")
    if initialized["serverInfo"].get("version") != manifest["upstream_version"]:
        raise ValueError("Native helper runtime version does not match source manifest")
    tools = replies.get(2, {}).get("result", {}).get("tools", [])
    names = {tool["name"] for tool in tools}
    if names != TOOLS or len(tools) != len(TOOLS):
        raise ValueError(f"Native tool contract mismatch: received {sorted(names)}")
    if any(tool.get("inputSchema", {}).get("type") != "object" for tool in tools):
        raise ValueError("Native tool has no object input schema")
    print(json.dumps({"target": manifest["target"], "server": replies[1]["result"]["serverInfo"],
                      "tools": sorted(names), "desktop_operations_performed": False}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    check(parser.parse_args().directory.resolve())
