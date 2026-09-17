from __future__ import annotations

import json
import re
import sqlite3
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SPEC_FILE = ROOT / "docs/agent-drafter/testing/hundred-app-test-specs.md"
STORE = ROOT / ".br-testdrive/runtime/config/biorouter/agent_drafter"
SESSIONS = ROOT / ".br-testdrive/runtime/data/sessions/sessions.db"
BUILTIN_EXTENSIONS = {
    "developer",
    "computercontroller",
    "webdocuments",
    "autovisualiser",
    "memory",
    "agent_drafter",
    "knowledge",
}
AVAILABLE_EXTERNAL_EXTENSIONS: set[str] = set()
AVAILABLE_SKILLS: set[str] = set()
AVAILABLE_KBS: set[str] = set()


def parse_specs() -> dict[int, dict[str, str]]:
    text = SPEC_FILE.read_text()
    matches = list(re.finditer(r"(?m)^### (\d+)\. (.+)$", text))
    specs: dict[int, dict[str, str]] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        block = text[match.start() : end]
        integration = re.search(r"(?m)^\*\*Platform integration:\*\*\s*(.+)$", block)
        specs[int(match.group(1))] = {
            "title": match.group(2).strip(),
            "platform": integration.group(1) if integration else "",
        }
    return specs


def request_flags(line: str) -> dict[str, bool]:
    lowered = line.lower()
    return {
        "knowledge_base": "kb " in lowered or lowered.startswith("kb") or "knowledge base" in lowered,
        "skill": "skill" in lowered,
        "extension": "extension" in lowered,
        "connector": "connector" in lowered,
        "model_route": "model route" in lowered,
        "workflow": "workflow" in lowered,
        "scientific_figure": "scientific figure" in lowered,
        "export": "export" in lowered,
    }


def runtime_tools() -> dict[str, set[str]]:
    found: dict[str, set[str]] = {}
    if not SESSIONS.exists():
        return found
    connection = sqlite3.connect(SESSIONS)
    rows = connection.execute(
        "select s.name, m.content_json from messages m "
        "join sessions s on s.id=m.session_id where s.name like 'app:spec-%'"
    ).fetchall()
    for name, raw in rows:
        try:
            parts = json.loads(raw)
        except json.JSONDecodeError:
            continue
        app_name = name.split(":", 2)[:2]
        app_key = ":".join(app_name)
        for part in parts:
            tool_call = part.get("toolCall") or {}
            value = tool_call.get("value") if isinstance(tool_call, dict) else None
            tool = value.get("name") if isinstance(value, dict) else None
            if tool:
                found.setdefault(app_key, set()).add(tool)
    return found


def knowledge_grants(agent: dict[str, Any]) -> set[str]:
    sources = (((agent.get("capabilities") or {}).get("data") or {}).get("sources") or [])
    return {
        identifier
        for source in sources
        if isinstance(source, dict) and source.get("kind") == "knowledge"
        for identifier in source.get("ids") or []
    }


def audit_app(number: int, spec: dict[str, str], tools: dict[str, set[str]]) -> dict[str, Any]:
    candidates = list(STORE.glob(f"spec-{number:03d}-*"))
    if not candidates:
        return {
            "number": number,
            "title": spec["title"],
            "built": False,
            "requested": request_flags(spec["platform"]),
            "platform_line": spec["platform"],
        }
    app = candidates[0]
    manifest = json.loads((app / "manifest.json").read_text())
    source = (app / "src/main.ts").read_text(errors="replace")
    agent = manifest.get("agent") or {}
    orchestration = agent.get("orchestration") or {}
    extensions = set(agent.get("extensions") or [])
    skills = set(agent.get("skills") or [])
    kb = agent.get("knowledge_base")
    grants = knowledge_grants(agent)
    runtime = tools.get(f"app:{app.name}", set())
    integration_tools = sorted(
        tool for tool in runtime if any(key in tool for key in ("knowledge", "skills", "workflow", "figure", "export"))
    )
    requested = request_flags(spec["platform"])
    issues: list[str] = []
    if requested["knowledge_base"] and not AVAILABLE_KBS:
        issues.append("requested KB capability unavailable in isolated catalog")
    if requested["skill"] and not AVAILABLE_SKILLS:
        issues.append("requested skill capability unavailable in isolated catalog")
    if requested["connector"] and not AVAILABLE_EXTERNAL_EXTENSIONS:
        issues.append("requested connector capability unavailable in isolated catalog")
    unknown_extensions = sorted(extensions - BUILTIN_EXTENSIONS - AVAILABLE_EXTERNAL_EXTENSIONS)
    if unknown_extensions:
        issues.append("unavailable extensions/connectors: " + ", ".join(unknown_extensions))
    if skills - AVAILABLE_SKILLS:
        issues.append("unavailable skills: " + ", ".join(sorted(skills - AVAILABLE_SKILLS)))
    if kb and kb not in AVAILABLE_KBS:
        issues.append(f"unavailable knowledge_base: {kb}")
    if grants - AVAILABLE_KBS:
        issues.append("unavailable KB grants: " + ", ".join(sorted(grants - AVAILABLE_KBS)))
    routes = orchestration.get("routes") or {}
    workflows = orchestration.get("workflows") or {}
    if requested["model_route"] and not routes:
        issues.append("requested model routes are not configured")
    if requested["workflow"] and not workflows:
        issues.append("requested workflow is not configured")
    if requested["scientific_figure"] and "autovisualiser" not in extensions:
        issues.append("requested scientific figures lack autovisualiser")
    return {
        "number": number,
        "id": app.name,
        "title": spec["title"],
        "built": True,
        "platform_line": spec["platform"],
        "requested": requested,
        "configured": {
            "extensions": sorted(extensions),
            "skills": sorted(skills),
            "knowledge_base": kb,
            "knowledge_grants": sorted(grants),
            "routes": sorted(routes),
            "workflows": sorted(workflows),
        },
        "available": {
            "extensions": sorted(extensions & BUILTIN_EXTENSIONS),
            "external_extensions_connectors": sorted(extensions & AVAILABLE_EXTERNAL_EXTENSIONS),
            "skills": sorted(skills & AVAILABLE_SKILLS),
            "knowledge_bases": sorted(({kb} if kb else set()) & AVAILABLE_KBS | grants & AVAILABLE_KBS),
        },
        "exercised": {
            "integration_tools": integration_tools,
            "route_referenced_in_client": bool(routes) and "route" in source,
            "workflow_tool_seen": any("workflow" in tool for tool in runtime),
        },
        "issues": issues,
    }


def main() -> None:
    specs = parse_specs()
    tools = runtime_tools()
    results = [audit_app(number, specs[number], tools) for number in range(1, 101)]
    built = [result for result in results if result["built"]]
    output = {
        "catalog": {
            "builtin_extensions": sorted(BUILTIN_EXTENSIONS),
            "external_extensions_connectors": [],
            "skills": [],
            "knowledge_bases": [],
        },
        "summary": {
            "built": len(built),
            "with_integration_issues": sum(bool(result.get("issues")) for result in built),
            "with_runtime_integration_tool_attempts": sum(
                bool((result.get("exercised") or {}).get("integration_tools")) for result in built
            ),
        },
        "results": results,
    }
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
