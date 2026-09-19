#!/usr/bin/env python3
"""Drive Agent Drafter through the locked 100-app Apps SDK v2 corpus.

The script never authors app files. It sends complete specs and review findings
to Biorouter's Agent Drafter, which must use create_app/update_app/configure_app/
build_app. All persistent runtime state is rooted under .br-testdrive/.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import unicodedata
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SPEC_FILE = ROOT / "docs/agent-drafter/testing/hundred-app-test-specs.md"
OUT = ROOT / "docs/agent-drafter-testdrive-100"
RUNTIME = ROOT / ".br-testdrive/runtime"
STORE = RUNTIME / "config/biorouter/agent_drafter"
LOGS = OUT / "authoring-logs"
RESULTS = OUT / "results"
LEDGER = OUT / "ledger.json"
CLI = Path(os.environ.get("BIOROUTER_TESTDRIVE_CLI", "/tmp/br-testdrive-target/debug/biorouter"))
PROVIDER = "versa_azure"
MODEL = "gpt-5.5-2026-04-24"
BUILTIN_EXTENSIONS = {
    "developer",
    "computercontroller",
    "webdocuments",
    "autovisualiser",
    "memory",
    "agent_drafter",
    "knowledge",
}
# The isolated test runtime intentionally starts with no user-installed payload.
# Keep these catalogs explicit so Drafter cannot earn integration credit by
# inventing identifiers that do not exist in the environment under test.
AVAILABLE_EXTERNAL_EXTENSIONS: set[str] = set()
AVAILABLE_SKILLS: set[str] = set()
AVAILABLE_KBS: set[str] = set()
PROVIDER_ERROR_MARKERS = (
    "Authentication failed. Status: 403 Forbidden",
    "The IP Address is invalid",
)


class ProviderUnavailable(RuntimeError):
    pass


def is_provider_error(output: str) -> bool:
    return any(marker in output for marker in PROVIDER_ERROR_MARKERS)


def credited_rounds(rounds: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [round_ for round_ in rounds if not round_.get("provider_error")]


def parse_specs() -> list[dict[str, Any]]:
    text = SPEC_FILE.read_text()
    matches = list(re.finditer(r"(?m)^### (\d+)\. (.+)$", text))
    specs: list[dict[str, Any]] = []
    for i, match in enumerate(matches):
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        specs.append(
            {
                "number": int(match.group(1)),
                "title": match.group(2).strip(),
                "block": text[match.start() : end].strip(),
            }
        )
    if [s["number"] for s in specs] != list(range(1, 101)):
        raise RuntimeError("spec corpus must contain exactly the ordered headings 1..100")
    return specs


def slugify(value: str) -> str:
    value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode()
    value = re.sub(r"[^a-zA-Z0-9]+", "-", value).strip("-").lower()
    return value[:48].rstrip("-")


def app_id(spec: dict[str, Any]) -> str:
    return f"spec-{spec['number']:03d}-{slugify(spec['title'])}"


def ensure_dirs() -> None:
    for path in (STORE, LOGS, RESULTS, OUT / "shots"):
        path.mkdir(parents=True, exist_ok=True)


def env() -> dict[str, str]:
    result = dict(os.environ)
    result.update(
        {
            "BIOROUTER_PATH_ROOT": str(RUNTIME),
            # Agent Drafter's default_root() currently bypasses Paths and reads
            # etcetera/XDG directly, so both variables are required for a truly
            # isolated store. This is audited as an SDK ergonomics finding.
            "XDG_CONFIG_HOME": str(RUNTIME / "config"),
            "BIOROUTER_PROVIDER": PROVIDER,
            "BIOROUTER_MODEL": MODEL,
            "BIOROUTER_DISABLE_KEYRING": "true",
            "BIOROUTER_ESBUILD_BIN": str(ROOT / "ui/desktop/node_modules/.bin/esbuild"),
            "CARGO_TARGET_DIR": "/tmp/br-testdrive-target",
        }
    )
    if not result.get("VERSA_AZURE_API_KEY"):
        raise RuntimeError("VERSA_AZURE_API_KEY is not set")
    return result


BUILD_ORDER = """Build one Biorouter Apps SDK v2 application to the exact specification below.

NON-NEGOTIABLE AUTHORING RULES:
- Use the exact app id: {id}
- The app must be kind=agentic and must use a non-chat archetype fitting the spec.
- You, Biorouter Agent Drafter, must author every app file through create_app,
  update_app, configure_app, and build_app. Do not use shell/file tools and do
  not merely explain how to build it.
- Preserve the complete ambitious specification. Do not simplify it.
- Declare every listed action (with typed params), signal, custom component, and
  a useful state_schema in manifest.surface; register/emit every declaration in
  src/main.ts.
- Implement the named regions, pixel-size intent, direct manipulation controls,
  bindings, presence narration, signal loop, and worker profiles. The primary
  surface must not be a chat transcript; any composer must be small/secondary.
- Treat the spec's regions as a functional contract, not a license to reuse a
  generic three-column shell. Give this app a composition, hierarchy, spatial
  rhythm, responsive behavior, and control language specific to its concept.
  Do not copy prior apps' rail/card/inspector CSS. Where the locked spec requires
  left/center/right regions, make those regions visually and interactively
  distinct while preserving their exact roles and pixel-size intent.
- The app's main agent, every orchestration.agents worker, every sub-agent, and
  every fast/deep model route MUST use provider "versa_azure" and model
  "gpt-5.5-2026-04-24". Never select or mention another model/provider in the
  manifest. This is the UCSF Azure OpenAI deployment required for this run.
- The runtime agent's system prompt must call ui_describe first, subscribe to
  declared signals, consult at least two named profiles, perform multiple
  app_call/ui_patch steps, mutate shared state, and narrate each step.
- Give worker profiles lowercase stable manifest keys. The main prompt must use
  those exact keys in consult calls. Explicitly set every consulted worker's
  ui.enabled=false so only the main agent controls the page.
- Call ui_describe only once per turn, then call ui_subscribe with every declared
  signal even if describe is unavailable; never repeat an unchanged describe.
- Mount the SDK run-status timeline into the specified visible progress region.
- `br.kb` is an API namespace, not a knowledge-base id. Never invent it as an
  id; any configured KB id must be a valid lowercase a-z/0-9/hyphen slug.
- PLATFORM CATALOG FOR THIS ISOLATED RUN: available built-in extensions are
  developer, computercontroller, webdocuments, autovisualiser, memory, agent_drafter, and
  knowledge. Available external extensions/connectors: none.
  Installed runtime skills: none. Installed knowledge bases: none. Configure
  only real catalog entries. Never invent a skill, KB, connector, or extension
  id to satisfy the spec. When a requested KB/skill/connector is unavailable,
  leave its id field unset (knowledge data-source ids empty) and preserve the
  unmet requirement in the agent prompt/documentation instead of pretending it
  exists. Use the real `knowledge`/`autovisualiser` built-ins when requested.
- Treat the Platform integration line as executable scope: when it asks for
  model routes or workflows, declare usable orchestration.routes/workflows on
  the locked UCSF model. When it asks for scientific figures, enable the real
  autovisualiser built-in. Do not claim an export or connector ran unless it did.
- Use theme tokens and the exact requested shipped pack. For terminal/midnight,
  call createApp with theme:"auto" so the requested dark ground is visible.
- Build the app, run lint through build_app, fix every ERROR, rebuild, and stop.
  Do not launch or export it. Finish by reporting the exact app id and lint result.
- First call create_app with the exact id, archetype, model, and WITHOUT the
  orchestration field so a valid starter definitely exists. Then read its
  manifest and update manifest.json with its required metadata preserved
  (including created_at/updated_at), adding surface/theme/orchestration. This
  avoids guessing the deeply nested orchestration schema in create_app.

FULL LOCKED SPECIFICATION:
{block}
"""


FIX_ORDER = """Continue refining app {id} in this same Agent Drafter conversation.
The static/browser reviewer found the following exact gaps:

{issues}

Fix every gap through Agent Drafter tools only. Preserve working features and the
full locked spec. Re-read manifest.json/index.html/src/main.ts as needed, update
them through update_app/configure_app, then build_app and fix every lint ERROR.
If the app is absent, first create a valid starter with create_app using the exact
id/archetype/model and omit orchestration; only after creation, read and extend
the manifest while preserving required metadata such as created_at/updated_at.
Keep the main agent, all workers/sub-agents, and every model route pinned only to
versa_azure / gpt-5.5-2026-04-24. Do not launch or export. Report the changes.
"""


def run_turn(spec: dict[str, Any], prompt: str, resume: bool, kind: str) -> dict[str, Any]:
    ident = app_id(spec)
    args = [
        str(CLI),
        "run",
        "--with-builtin",
        "agent_drafter",
        "-n",
        f"testdrive-{ident}",
        "--max-turns",
        "80",
        "-i",
        "-",
    ]
    if resume:
        args.append("--resume")
    started = time.time()
    try:
        proc = subprocess.run(
            args,
            cwd=ROOT,
            env=env(),
            input=prompt,
            text=True,
            capture_output=True,
            timeout=900,
        )
        output = (proc.stdout or "") + "\n--- STDERR ---\n" + (proc.stderr or "")
        rc = proc.returncode
    except subprocess.TimeoutExpired as exc:
        output = (exc.stdout or "") + "\n--- TIMEOUT ---\n" + (exc.stderr or "")
        rc = -1
    duration = round(time.time() - started, 1)
    provider_error = is_provider_error(output)
    if provider_error:
        rc = 75
    with (LOGS / f"{ident}.log").open("a") as handle:
        handle.write(f"\n\n===== {kind} rc={rc} duration={duration}s =====\n{output}")
    return {
        "kind": "provider-blocked" if provider_error else kind,
        "rc": rc,
        "duration_s": duration,
        "provider_error": provider_error,
    }


def names_from_line(block: str, label: str) -> list[str]:
    match = re.search(rf"(?m)^\*\*{re.escape(label)}[^\n]*:\*\*\s*(.+)$", block)
    if not match:
        return []
    line = match.group(1)
    return re.findall(r"`?([a-z][a-z0-9_]*)`?\s*\(", line, re.I)


def agent_names(block: str) -> list[str]:
    match = re.search(r"(?m)^\*\*Agents \(multi-agent\):\*\*\s*(.+)$", block)
    return re.findall(r"\*([^*]+)\*", match.group(1)) if match else []


def expected_pack(block: str) -> str | None:
    match = re.search(r"(?m)^\*\*Theme & aesthetic:\*\*.*?`([a-z-]+)`", block)
    return match.group(1) if match else None


def platform_line(block: str) -> str:
    match = re.search(r"(?m)^\*\*Platform integration:\*\*\s*(.+)$", block)
    return match.group(1) if match else ""


def resolved_theme_pack(manifest: dict[str, Any]) -> str:
    # Manifest serialization intentionally omits the default ThemeConfig. An
    # absent theme block therefore means the shipped base `biorouter` pack.
    return (manifest.get("theme") or {}).get("pack") or "biorouter"


def model_refs(value: Any, path: str = "") -> list[tuple[str, dict[str, Any]]]:
    found: list[tuple[str, dict[str, Any]]] = []
    if isinstance(value, dict):
        if isinstance(value.get("provider"), str) or isinstance(value.get("model"), str):
            found.append((path or "/", value))
        for key, child in value.items():
            found.extend(model_refs(child, f"{path}/{key}"))
    elif isinstance(value, list):
        for idx, child in enumerate(value):
            found.extend(model_refs(child, f"{path}/{idx}"))
    return found


def static_review(spec: dict[str, Any]) -> dict[str, Any]:
    ident = app_id(spec)
    app = STORE / ident
    issues: list[str] = []
    manifest_path = app / "manifest.json"
    if not manifest_path.exists():
        return {"id": ident, "built": False, "issues": ["manifest.json was not created"]}
    try:
        manifest = json.loads(manifest_path.read_text())
    except Exception as exc:
        return {"id": ident, "built": False, "issues": [f"manifest invalid: {exc}"]}
    html = (app / "index.html").read_text(errors="replace") if (app / "index.html").exists() else ""
    source = (app / "src/main.ts").read_text(errors="replace") if (app / "src/main.ts").exists() else ""
    built = (app / "dist/app.js").exists() and (app / "dist/app.js").stat().st_size > 500
    if not built:
        issues.append("dist/app.js is missing or too small; build_app did not produce a usable bundle")
    if manifest.get("kind") != "agentic":
        issues.append(f"manifest.kind is {manifest.get('kind')!r}, expected 'agentic'")
    if manifest.get("archetype") == "chat":
        issues.append("chat archetype violates the spec's non-chat primary-surface requirement")

    surface = manifest.get("surface") or {}
    actual_actions = {x.get("name") for x in surface.get("actions", []) if isinstance(x, dict)}
    actual_signals = {x.get("name") for x in surface.get("signals", []) if isinstance(x, dict)}
    expected_actions = set(names_from_line(spec["block"], "Declared actions"))
    expected_signals = set(names_from_line(spec["block"], "Signals"))
    missing_actions = sorted(expected_actions - actual_actions)
    missing_signals = sorted(expected_signals - actual_signals)
    if missing_actions:
        issues.append("manifest.surface.actions is missing: " + ", ".join(missing_actions))
    if missing_signals:
        issues.append("manifest.surface.signals is missing: " + ", ".join(missing_signals))
    unregistered = sorted(name for name in expected_actions if f'"{name}"' not in source)
    unemitted = sorted(name for name in expected_signals if f'"{name}"' not in source)
    if unregistered:
        issues.append("src/main.ts does not reference/register actions: " + ", ".join(unregistered))
    if unemitted:
        issues.append("src/main.ts does not reference/emit signals: " + ", ".join(unemitted))
    if not surface.get("state_schema"):
        issues.append("manifest.surface.state_schema is absent")
    if "data-br-bind" not in html:
        issues.append("index.html has no data-br-bind reactive binding")
    regions = re.findall(r'data-br-region=["\']([^"\']+)', html)
    if len(set(regions)) < 3:
        issues.append(f"only {len(set(regions))} named regions found; the specified multi-panel layout needs at least 3")
    if "createApp({" not in source or "autoChat: false" not in source:
        issues.append("src/main.ts must create a purpose-built UI with createApp({ autoChat: false })")
    if "ui_describe" not in json.dumps(manifest.get("agent", {})):
        issues.append("runtime system prompt does not explicitly require ui_describe first")
    if "consult" not in json.dumps(manifest.get("agent", {})):
        issues.append("runtime system prompt does not explicitly orchestrate worker consults")

    expected_agents = agent_names(spec["block"])
    actual_agents = ((manifest.get("agent") or {}).get("orchestration") or {}).get("agents") or {}
    if len(actual_agents) < 2:
        issues.append(f"only {len(actual_agents)} orchestration.agents profiles declared; expected at least 2 ({', '.join(expected_agents)})")
    normalized_actual = {slugify(name).replace("-", "") for name in actual_agents}
    missing_profiles = [name for name in expected_agents if slugify(name).replace("-", "") not in normalized_actual]
    if missing_profiles:
        issues.append("named worker profiles not declared recognizably: " + ", ".join(missing_profiles))

    pack = expected_pack(spec["block"])
    actual_pack = resolved_theme_pack(manifest)
    if pack and actual_pack != pack:
        issues.append(f"theme.pack resolves to {actual_pack!r}; expected {pack!r}")
    if pack in {"terminal", "midnight"} and 'theme: "auto"' not in source and "theme:'auto'" not in source:
        issues.append(f"{pack} app does not opt into createApp theme:'auto', so dark grounds may not render")

    agent = manifest.get("agent") or {}
    integration = platform_line(spec["block"]).lower()
    configured_extensions = set(agent.get("extensions") or [])
    unknown_extensions = sorted(
        configured_extensions - BUILTIN_EXTENSIONS - AVAILABLE_EXTERNAL_EXTENSIONS
    )
    if unknown_extensions:
        issues.append("unavailable extension/connector ids configured: " + ", ".join(unknown_extensions))
    unavailable_skills = sorted(set(agent.get("skills") or []) - AVAILABLE_SKILLS)
    if unavailable_skills:
        issues.append("unavailable skill ids configured: " + ", ".join(unavailable_skills))
    knowledge_base = agent.get("knowledge_base")
    if knowledge_base and knowledge_base not in AVAILABLE_KBS:
        issues.append(f"unavailable knowledge_base configured: {knowledge_base}")
    data_sources = ((agent.get("capabilities") or {}).get("data") or {}).get("sources") or []
    granted_kbs = {
        identifier
        for source in data_sources
        if isinstance(source, dict) and source.get("kind") == "knowledge"
        for identifier in source.get("ids") or []
    }
    missing_granted_kbs = sorted(granted_kbs - AVAILABLE_KBS)
    if missing_granted_kbs:
        issues.append("unavailable knowledge grant ids configured: " + ", ".join(missing_granted_kbs))
    if "kb " in integration or integration.startswith("kb") or "knowledge base" in integration:
        if "knowledge" not in configured_extensions:
            issues.append("platform integration requests a KB but the real knowledge extension is not enabled")
    if "scientific figure" in integration and "autovisualiser" not in configured_extensions:
        issues.append("platform integration requests scientific figures but autovisualiser is not enabled")
    orchestration = agent.get("orchestration") or {}
    if "model route" in integration and not (orchestration.get("routes") or {}):
        issues.append("platform integration requests model routes but orchestration.routes is empty")
    if "workflow" in integration and not (orchestration.get("workflows") or {}):
        issues.append("platform integration requests a workflow but orchestration.workflows is empty")
    main_model = agent.get("model") or {}
    if main_model.get("provider") != PROVIDER or main_model.get("model") != MODEL:
        issues.append(f"main agent model must be exactly {PROVIDER}/{MODEL}, found {main_model}")
    for path, ref in model_refs(agent.get("orchestration") or {}, "/agent/orchestration"):
        if ref.get("provider") != PROVIDER or ref.get("model") != MODEL:
            issues.append(f"non-UCSF model reference at {path}: {ref}")

    return {
        "id": ident,
        "title": spec["title"],
        "built": built,
        "manifest": str(manifest_path.relative_to(ROOT)),
        "actions_expected": sorted(expected_actions),
        "actions_actual": sorted(x for x in actual_actions if x),
        "signals_expected": sorted(expected_signals),
        "signals_actual": sorted(x for x in actual_signals if x),
        "agents_expected": expected_agents,
        "agents_actual": sorted(actual_agents),
        "regions": sorted(set(regions)),
        "theme_expected": pack,
        "theme_actual": actual_pack,
        "issues": issues,
    }


def load_ledger() -> dict[str, Any]:
    return json.loads(LEDGER.read_text()) if LEDGER.exists() else {}


def save_ledger(value: dict[str, Any]) -> None:
    LEDGER.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def write_preliminary_result(spec: dict[str, Any], review: dict[str, Any], rounds: int) -> None:
    ident = app_id(spec)
    manifest = json.loads((STORE / ident / "manifest.json").read_text()) if (STORE / ident / "manifest.json").exists() else {}
    agent = manifest.get("agent") or {}
    orchestration = agent.get("orchestration") or {}
    extensions = sorted(agent.get("extensions") or [])
    skills = sorted(agent.get("skills") or [])
    grants = sorted(
        identifier
        for source in ((((agent.get("capabilities") or {}).get("data") or {}).get("sources") or []))
        if isinstance(source, dict) and source.get("kind") == "knowledge"
        for identifier in source.get("ids") or []
    )
    available_extensions = sorted(set(extensions) & BUILTIN_EXTENSIONS)
    requested_integration = platform_line(spec["block"])
    configured_line = (
        f"extensions={extensions or 'none'}; skills={skills or 'none'}; "
        f"knowledge_base={agent.get('knowledge_base') or 'none'}; grants={grants or 'none'}; "
        f"routes={sorted((orchestration.get('routes') or {}).keys()) or 'none'}; "
        f"workflows={sorted((orchestration.get('workflows') or {}).keys()) or 'none'}"
    )
    issue_lines = "\n".join(f"- {issue}" for issue in review["issues"]) or "- None in static review."
    status = "PASS" if review["built"] and not review["issues"] else ("PARTIAL" if review["built"] else "FAIL")
    text = f"""# Spec {spec['number']:03d} — {spec['title']}
- **App id:** {ident}
- **Authoring rounds:** {rounds}   **Reached acceptance:** pending browser verification
- **Channel:** CLI (named resumable Biorouter session)
- **Provider/model:** `{PROVIDER}/{MODEL}` (UCSF Azure OpenAI)

## Functional verdict: {status} (static; browser pending)
| Check | Result | Notes |
|---|---|---|
| Not a chatbot (5.2) | pending | Browser verification required |
| Layout matches (5.3) | pending | Static regions: {', '.join(review.get('regions', [])) or 'none'} |
| Declared surface (5.4) | {'✅' if review['built'] and not any('surface' in x or 'actions' in x or 'signals' in x for x in review['issues']) else '❌'} | Static manifest/source cross-check |
| Client reactivity (5.5) | pending | Browser state/binding drive required |
| Agent-driven loop (5.6) | pending | Two live instructions required |
| Multi-agent ran (5.7) | pending | Profiles declared: {', '.join(review.get('agents_actual', [])) or 'none'} |
| Signals round-trip (5.8) | pending | Gesture + agent reaction required |

## Aesthetic verdict: PENDING
- Expected pack `{review.get('theme_expected')}`; manifest pack `{review.get('theme_actual')}`.

## Platform integration
- **Requested:** {requested_integration}
- **Configured:** {configured_line}
- **Available in isolated runtime:** built-in extensions={available_extensions or 'none'}; external connectors=none; skills=none; KBs=none.
- **Exercised:** pending browser/session verification. A configured name is not credited until a real runtime tool/route/workflow succeeds.
- **Missing/blocked:** requested skills, KB payloads, and external connectors are unavailable unless the catalog changes; invented ids are static failures.

## Screenshots
- `../shots/spec-{spec['number']:03d}-*.png` (pending)

## Friction encountered
{issue_lines}
"""
    (RESULTS / f"spec-{spec['number']:03d}.md").write_text(text)


def build_one(spec: dict[str, Any], max_rounds: int) -> dict[str, Any]:
    ident = app_id(spec)
    ledger = load_ledger()
    entry = ledger.setdefault(ident, {"number": spec["number"], "title": spec["title"], "rounds": []})
    resume = bool(entry["rounds"])
    usable_rounds = credited_rounds(entry["rounds"])
    if not (STORE / ident / "manifest.json").exists():
        prompt = BUILD_ORDER.format(id=ident, block=spec["block"])
        result = run_turn(spec, prompt, resume, "build")
        entry["rounds"].append(result)
        save_ledger(ledger)
        if result["provider_error"]:
            raise ProviderUnavailable(
                f"UCSF Azure rejected {ident}; batch stopped before recording false build progress"
            )
        usable_rounds.append(result)

    review = static_review(spec)
    while review["issues"] and len(usable_rounds) < max_rounds:
        prompt = FIX_ORDER.format(id=ident, issues="\n".join(f"- {x}" for x in review["issues"]))
        result = run_turn(spec, prompt, True, "static-fix")
        entry["rounds"].append(result)
        save_ledger(ledger)
        if result["provider_error"]:
            raise ProviderUnavailable(
                f"UCSF Azure rejected {ident}; batch stopped before recording false fix progress"
            )
        usable_rounds.append(result)
        review = static_review(spec)

    entry["static_review"] = review
    save_ledger(ledger)
    (LOGS / f"{ident}-static.json").write_text(json.dumps(review, indent=2) + "\n")
    write_preliminary_result(spec, review, len(entry["rounds"]))
    return review


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("list")
    build = sub.add_parser("build")
    build.add_argument("--start", type=int, default=1)
    build.add_argument("--count", type=int, default=1)
    build.add_argument("--max-rounds", type=int, default=4)
    review = sub.add_parser("review")
    review.add_argument("number", type=int)
    fix = sub.add_parser("fix")
    fix.add_argument("number", type=int)
    fix.add_argument("issues")
    args = parser.parse_args()
    ensure_dirs()
    specs = parse_specs()
    if args.command == "list":
        for spec in specs:
            print(f"{spec['number']:03d} {app_id(spec)}")
        return
    if args.command == "review":
        print(json.dumps(static_review(specs[args.number - 1]), indent=2))
        return
    if args.command == "fix":
        spec = specs[args.number - 1]
        issues = Path(args.issues[1:]).read_text() if args.issues.startswith("@") else args.issues
        result = run_turn(spec, FIX_ORDER.format(id=app_id(spec), issues=issues), True, "manual-fix")
        ledger = load_ledger()
        entry = ledger.setdefault(app_id(spec), {"number": spec["number"], "title": spec["title"], "rounds": []})
        entry["rounds"].append(result)
        entry["static_review"] = static_review(spec)
        save_ledger(ledger)
        if result["provider_error"]:
            raise ProviderUnavailable(
                f"UCSF Azure rejected {app_id(spec)}; no refinement was credited"
            )
        print(json.dumps(entry["static_review"], indent=2))
        return

    stop = min(101, args.start + args.count)
    failures = 0
    for number in range(args.start, stop):
        spec = specs[number - 1]
        print(f"[{number:03d}/100] {app_id(spec)}", flush=True)
        review_result = build_one(spec, args.max_rounds)
        if review_result["issues"]:
            failures += 1
        print(
            f"  built={review_result['built']} static_issues={len(review_result['issues'])}",
            flush=True,
        )
    print(f"batch complete: {stop - args.start} apps, {failures} with remaining static issues")


if __name__ == "__main__":
    try:
        main()
    except ProviderUnavailable as exc:
        print(f"provider unavailable: {exc}", file=sys.stderr)
        sys.exit(75)
    except KeyboardInterrupt:
        sys.exit(130)
