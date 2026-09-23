#!/usr/bin/env python3
"""Exercise the bundled stdio handshake without observing or controlling a desktop."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

TOOLS = {"list_apps", "get_app_state", "click", "perform_secondary_action", "scroll", "drag",
         "type_text", "press_key", "set_value", "screen_capture"}
CONTRACT = Path(__file__).resolve().parents[1] / "crates/biorouter-mcp/tests/fixtures/computer-use-tools.json"
RETIRED_PROMPT_NAME = re.compile(r"\b(?:open[ -]+computer[ -]+use|computer[ -]+controller|computer[ -]+use)\b", re.I)


def semantic_schema(value):
    if isinstance(value, dict):
        return {key: semantic_schema(item) for key, item in value.items()
                if key not in {"description", "title", "default", "$schema"}}
    if isinstance(value, list):
        return [semantic_schema(item) for item in value]
    return value


def validate_tools(tools):
    expected = json.loads(CONTRACT.read_text())["tools"]
    names = {tool["name"] for tool in tools}
    if names != TOOLS or len(tools) != len(TOOLS):
        raise ValueError(f"Native tool contract mismatch: received {sorted(names)}")
    for tool in expected:
        actual = next(item for item in tools if item["name"] == tool["name"])
        if RETIRED_PROMPT_NAME.search(actual.get("description", "")):
            raise ValueError(f"Retired Copilot name in native tool description: {tool['name']}")
        actual_schema = json.dumps(semantic_schema(actual.get("inputSchema")), sort_keys=True)
        expected_schema = json.dumps(tool["inputSchema"], sort_keys=True)
        if actual_schema != expected_schema:
            raise ValueError(f"Native schema mismatch: {tool['name']}")
    return names


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
    instructions = initialized.get("instructions", "")
    if "Biorouter Copilot" not in instructions or RETIRED_PROMPT_NAME.search(instructions):
        raise ValueError("Native helper instructions must name Biorouter Copilot")
    if initialized.get("protocolVersion") != "2025-03-26":
        raise ValueError("Native helper MCP protocol version drift")
    if initialized["serverInfo"].get("version") != manifest["upstream_version"]:
        raise ValueError("Native helper runtime version does not match source manifest")
    tools = replies.get(2, {}).get("result", {}).get("tools", [])
    names = validate_tools(tools)
    print(json.dumps({"target": manifest["target"], "server": replies[1]["result"]["serverInfo"],
                      "tools": sorted(names), "desktop_operations_performed": False}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    check(parser.parse_args().directory.resolve())
