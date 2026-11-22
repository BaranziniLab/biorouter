"""The start probe: launch an extension over stdio the way Biorouter does and list its tools.

Biorouter starts an installed bundle as ``uv run --directory <dir> <entry_point>``
and speaks MCP over the child's stdin and stdout. This probe does the same with
any command, completes the MCP handshake, lists the tools, calls the
``*_status`` tool if there is one, and returns what it saw. The child gets only
the environment it is handed (see ``clean_env``), never the caller's whole
environment, so a credential on the test machine cannot reach it by accident.

    python stdio_probe.py <cwd> <command> [args...]
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path
from typing import Any, Mapping, Optional, Sequence

# Variables a child process needs to start on every platform, and nothing else.
_PASSTHROUGH = (
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "COMSPEC",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "UV_CACHE_DIR",
    "UV_PYTHON_INSTALL_DIR",
    "XDG_CACHE_HOME",
    "APPDATA",
    "LOCALAPPDATA",
)


def clean_env(extra: Optional[Mapping[str, str]] = None) -> dict[str, str]:
    env = {k: os.environ[k] for k in _PASSTHROUGH if k in os.environ}
    env["PYTHONUNBUFFERED"] = "1"
    env["FASTMCP_SHOW_SERVER_BANNER"] = "false"
    env["FASTMCP_SHOW_CLI_BANNER"] = "false"
    if extra:
        env.update(extra)
    return env


async def _probe(command: Sequence[str], env: Mapping[str, str], cwd: Optional[str], log_file: Optional[Path]) -> dict[str, Any]:
    from .leak_suite import stdio_session

    async with stdio_session(command[0], command[1:], env=env, cwd=cwd, log_file=log_file) as session:
        tools = await session.list_tools()
        status_name = next((n for n in sorted(tools) if n.endswith("_status")), None)
        status_text = ""
        if status_name:
            call = await session.call(status_name, {})
            status_text = call.text
        info = session.server_info
        return {
            "server": getattr(info, "name", None),
            "version": getattr(info, "version", None),
            "instructions": session.instructions,
            "tools": sorted(tools),
            "annotations": {name: tools[name]["annotations"] for name in tools},
            "status_tool": status_name,
            "status_text": status_text,
        }


def probe(
    command: Sequence[str],
    *,
    env: Mapping[str, str],
    cwd: Optional[str] = None,
    log_file: Optional[Path] = None,
    timeout_s: float = 180.0,
) -> dict[str, Any]:
    async def run() -> dict[str, Any]:
        return await asyncio.wait_for(_probe(command, env, cwd, log_file), timeout_s)

    return asyncio.run(run())


def main(argv: Sequence[str]) -> int:
    if len(argv) < 3:
        print(__doc__)
        return 2
    cwd, command = argv[1], list(argv[2:])
    result = probe(command, env=clean_env(), cwd=cwd)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    if __package__ in (None, ""):
        sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
        __package__ = Path(__file__).resolve().parent.name
    raise SystemExit(main(sys.argv))
