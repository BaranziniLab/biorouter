#!/usr/bin/env python3
"""Iteratively build applications by driving GPT-5.5 through the drafter tools.

Each app is built inside ONE named session ("ad-<id>") so follow-up fixes land in
the SAME conversation — build -> review -> report problems -> it fixes -> repeat,
exactly like a human iterating with the model.

  build_batch.py build <prompts.json> <start> <count>   # round 1 + auto-fix build failures
  build_batch.py fix   <id> <issue-text-or-@file>        # resume the session, feed problems back
  build_batch.py rounds <id>                             # how many refine rounds an app has had

Sandboxed so the 100 apps live in a dedicated store. Per-app transcripts + a
rounds ledger under docs/agent-drafter-stress/build-logs/.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path("/Users/wgu/Desktop/biorouter-ad-stress")
SANDBOX = ROOT / ".ad-sandbox"
CLI = ROOT / "target/debug/biorouter"
STORE = SANDBOX / ".config/biorouter/agent_drafter"
ESBUILD = "/Users/wgu/Desktop/biorouter/ui/desktop/node_modules/.bin/esbuild"
RAW = ROOT / "docs/agent-drafter-stress/build-logs"
RAW.mkdir(parents=True, exist_ok=True)
LEDGER = RAW / "rounds.json"

ENV = {
    **os.environ,
    "HOME": str(SANDBOX),
    "XDG_CONFIG_HOME": str(SANDBOX / ".config"),
    "BIOROUTER_DISABLE_KEYRING": "true",
    "BIOROUTER_ESBUILD_BIN": ESBUILD,
}

WRAPPER = """Build ONE Biorouter app with the agent_drafter tools. Use this EXACT app id: "{id}".

REQUIREMENT:
{requirement}

HOW TO BUILD IT (do all of this, in order, then stop — do NOT launch):
1. create_app with id="{id}", a clear title and description, kind="agentic".
2. Write a self-contained, on-theme index.html (via `html` or update_app) that:
   - uses the injected Biorouter design system (br-* classes + CSS variables) — NO external CSS/JS/CDN;
   - colors ALL text with theme tokens (color: var(--br-text) / var(--br-text-muted)), NEVER hardcoded hex/rgb,
     so nothing goes invisible when themed; the app renders light by default;
   - lays out the regions the requirement needs as <section data-br-region="NAME">…</section>, each with a
     short placeholder line saying what will appear there;
   - exposes the concrete controls the requirement calls for (inputs, sliders <input type="range" class="br-slider">,
     selects, chips, buttons, drop zones, a map grid, etc.) with real ids;
   - has a visible progress surface (data-br-chat OR a br-run-status area).
3. Write src/main.ts that wires those controls to the agent: build the prompt from the control state and call
   br.run(prompt, target) / br.prompt(...) on change/click/submit, keeping the loop going across turns. Import ONLY
   from "./sdk". Show progress. GUARD every run: do NOT call br.run on page boot with an empty form, and skip the
   run when the user hasn't supplied the minimum input (e.g. `if (selected.length < 2) return;`) — handle the
   empty/partial state locally with a placeholder. Always pass the user's current selection/state INSIDE the prompt;
   never make the agent call ui_describe to discover what the user chose. If the app works through a list one item
   at a time (triage/quiz/resolve loops), the system_prompt MUST track resolved item-ids in ui_state, always pick the
   next UNRESOLVED item (never re-ask a finished one), append a numbered step to the log each turn, and define a clear
   stop condition — otherwise the loop re-asks the same top item forever.
4. configure_app (or in create_app) with a strong system_prompt telling the APP AGENT to DRIVE the interface every
   turn with the ui_* tools instead of answering in prose: which tool for which output (ui_panel/ui_render INTO the
   declared @region: regions — you can pass place="@region:NAME" to ui_panel; ui_chart/ui_graph for figures;
   ui_highlight to direct attention; ui_state to track the user's accumulated behavior; ui_ask for a structured
   choice mid-turn). Explicit and step-by-step. Set max_turns 12-20.
5. build_app. If the harness reports lint ERRORs, fix them and build_app again. Do NOT launch_app.

Aim for genuine visual craft and an interaction that actually loops. Finish by reporting the app id."""

FIX = """Keep iterating on the app "{id}" in this same session. A reviewer drove the built app in a browser
and found these problems. FIX them, then build_app again (do not launch):

{issues}

Make the smallest changes that fully resolve each point; preserve what already works. Update index.html /
src/main.ts / the system_prompt as needed via update_app / configure_app, then build_app. Report what you changed."""


def run_cli(instruction, session, resume):
    args = [str(CLI), "run", "--with-builtin", "agent_drafter", "-n", session,
            "--max-turns", "40", "-i", "-"]
    if resume:
        args.append("--resume")
    t0 = time.time()
    try:
        proc = subprocess.run(args, input=instruction, env=ENV, cwd=str(ROOT),
                              capture_output=True, text=True, timeout=720)
        out = (proc.stdout or "") + "\n---STDERR---\n" + (proc.stderr or "")
        rc = proc.returncode
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or "") + "\n---TIMEOUT---\n" + (e.stderr or "")
        rc = -1
    return rc, round(time.time() - t0, 1), out


def built(app_id):
    d = STORE / app_id
    return (d / "dist/app.js").exists() and (d / "manifest.json").exists()


def ledger_load():
    return json.loads(LEDGER.read_text()) if LEDGER.exists() else {}


def ledger_bump(app_id, kind, rc, dur, ok):
    led = ledger_load()
    e = led.setdefault(app_id, {"rounds": 0, "history": []})
    e["rounds"] += 1
    e["history"].append({"kind": kind, "rc": rc, "dur_s": dur, "built": ok})
    LEDGER.write_text(json.dumps(led, indent=2))
    return e["rounds"]


def append_log(app_id, header, out):
    p = RAW / f"{app_id}.log"
    with p.open("a") as f:
        f.write(f"\n\n===== {header} =====\n{out}")


def cmd_build(prompts_path, start, count):
    prompts = json.loads(Path(prompts_path).read_text())
    batch = prompts[start:start + count]
    results = []
    for i, p in enumerate(batch):
        app_id = p["id"]
        session = f"ad-{app_id}"
        print(f"[{start+i+1}] build {app_id} …", flush=True)
        instr = WRAPPER.format(id=app_id, requirement=p["requirement"].strip())
        rc, dur, out = run_cli(instr, session, resume=False)
        append_log(app_id, f"round1 build rc={rc} {dur}s", out)
        ok = built(app_id)
        rounds = ledger_bump(app_id, "build", rc, dur, ok)
        # One automatic recovery round if the build didn't land.
        if not ok:
            print(f"    round1 did not produce a bundle; auto-retry …", flush=True)
            fixmsg = FIX.format(id=app_id, issues=(
                "The app did not build into a servable bundle (no dist/app.js). Re-run create_app if needed, "
                "make sure src/main.ts imports from \"./sdk\" and index.html has no external assets, then build_app."))
            rc2, dur2, out2 = run_cli(fixmsg, session, resume=True)
            append_log(app_id, f"round2 autofix rc={rc2} {dur2}s", out2)
            ok = built(app_id)
            rounds = ledger_bump(app_id, "autofix", rc2, dur2, ok)
        print(f"    {'ok' if ok else 'FAIL'} built={ok} rounds={rounds}", flush=True)
        results.append({"id": app_id, "built": ok, "rounds": rounds})
    out_path = RAW / f"batch-{start}-{start+count}.json"
    out_path.write_text(json.dumps(results, indent=2))
    ok = sum(1 for r in results if r["built"])
    print(f"\nBATCH {start}-{start+count}: {ok}/{len(results)} built. -> {out_path}")


def cmd_fix(app_id, issue_arg):
    issues = Path(issue_arg[1:]).read_text() if issue_arg.startswith("@") else issue_arg
    session = f"ad-{app_id}"
    print(f"fix {app_id} (round via same session) …", flush=True)
    rc, dur, out = run_cli(FIX.format(id=app_id, issues=issues), session, resume=True)
    append_log(app_id, f"fix rc={rc} {dur}s", out)
    ok = built(app_id)
    rounds = ledger_bump(app_id, "fix", rc, dur, ok)
    print(f"    {'ok' if ok else 'FAIL'} built={ok} rounds={rounds}")
    # echo the model's closing report
    print("--- model report tail ---")
    print("\n".join(out.splitlines()[-12:]))


def main():
    cmd = sys.argv[1]
    if cmd == "build":
        cmd_build(sys.argv[2], int(sys.argv[3]), int(sys.argv[4]))
    elif cmd == "fix":
        cmd_fix(sys.argv[2], sys.argv[3])
    elif cmd == "rounds":
        print(json.dumps(ledger_load().get(sys.argv[2], {}), indent=2))
    else:
        sys.exit("usage: build_batch.py build|fix|rounds …")


if __name__ == "__main__":
    main()
