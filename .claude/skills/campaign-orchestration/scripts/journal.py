#!/usr/bin/env python3
"""Read-only helper for Claude Code Workflow journals.

A workflow run writes <run dir>/journal.jsonl with one line per event:
  {"type": "started", "key": ..., "agentId": ..., "label": ..., "phase": ...}
  {"type": "result",  "key": ..., "agentId": ..., "result": {...}}
  {"type": "failed",  "key": ..., "agentId": ...}
and one transcript per agent, <run dir>/agent-<agentId>.jsonl.

Results are joined to their label by agentId (falling back to key), never by
position: completion order is not launch order.

Usage:
  journal.py status RUN [RUN ...]            one line per agent: phase, state, idle minutes
  journal.py results RUN [--label PREFIX]    label plus a compact view of each result
  journal.py field RUN LABEL FIELD           print one result field (for example a report_md
                                             a subagent could not write) to stdout
  journal.py export RUN OUT.json             done results, failed and unfinished labels, for a
                                             continuation script (a PAUSE-STATE file)

RUN is a run id (wf_...) or a path to a run directory or journal.jsonl. A bare run id is
looked up under ~/.claude/projects/*/*/subagents/workflows/. Never writes anything except
the OUT.json that export is given.
"""
import glob
import json
import os
import sys
import time


def run_dir(run):
    if os.path.isfile(run):
        return os.path.dirname(os.path.abspath(run))
    if os.path.isdir(run):
        return os.path.abspath(run)
    pattern = os.path.expanduser(f"~/.claude/projects/*/*/subagents/workflows/{run}")
    hits = sorted(glob.glob(pattern), key=os.path.getmtime)
    if not hits:
        sys.exit(f"no run directory found for {run}")
    return hits[-1]


def load(run):
    d = run_dir(run)
    agents = {}  # agentId or key -> record
    by_key = {}
    with open(os.path.join(d, "journal.jsonl")) as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                continue
            t = e.get("type")
            aid, key = e.get("agentId"), e.get("key")
            if t == "started":
                rec = {"label": e.get("label", "?"), "phase": e.get("phase", ""),
                       "agentId": aid, "key": key, "state": "running", "result": None}
                agents[aid or key] = rec
                if key:
                    by_key[key] = rec
            elif t in ("result", "failed", "skipped"):
                rec = agents.get(aid) or by_key.get(key)
                if rec is None:
                    rec = {"label": "?", "phase": "", "agentId": aid, "key": key}
                    agents[aid or key] = rec
                rec["state"] = "done" if t == "result" else t
                r = e.get("result", e.get("value"))
                if isinstance(r, str):
                    try:
                        r = json.loads(r)
                    except ValueError:
                        pass
                rec["result"] = r
    return d, list(agents.values())


def idle_minutes(d, aid):
    files = glob.glob(os.path.join(d, f"agent-{aid}*.jsonl")) if aid else []
    if not files:
        return None
    return (time.time() - max(os.path.getmtime(p) for p in files)) / 60


def compact(r):
    if not isinstance(r, dict):
        return str(r)[:160]
    parts = []
    for k in ("status", "verdict", "pass", "confident", "lane"):
        if k in r:
            parts.append(f"{k}={r[k]}")
    f = r.get("findings")
    if isinstance(f, list):
        sev = {}
        for x in f:
            s = x.get("severity", "?") if isinstance(x, dict) else "?"
            sev[s] = sev.get(s, 0) + 1
        parts.append("findings=" + ",".join(f"{k}:{v}" for k, v in sorted(sev.items())) if sev else "findings=0")
    if "summary" in r and isinstance(r["summary"], str):
        parts.append("summary=" + r["summary"][:120].replace("\n", " "))
    return " ".join(parts) or ", ".join(list(r)[:8])


def cmd_status(runs):
    for run in runs:
        d, recs = load(run)
        print(f"# {os.path.basename(d)}")
        for r in recs:
            idle = idle_minutes(d, r.get("agentId")) if r["state"] == "running" else None
            tail = f" idle {idle:.0f} min" if idle is not None else ""
            print(f"{r['state']:8} {r['phase'][:16]:16} {r['label']}{tail}")


def cmd_results(run, prefix):
    _, recs = load(run)
    for r in recs:
        if prefix and not r["label"].startswith(prefix):
            continue
        if r["state"] != "running":
            print(f"{r['label']}: {r['state']} {compact(r['result'])}")


def cmd_field(run, label, field):
    _, recs = load(run)
    hits = [r for r in recs if r["label"] == label and isinstance(r.get("result"), dict)]
    if not hits:
        sys.exit(f"no finished result labelled {label!r}")
    v = hits[-1]["result"].get(field)
    if v is None:
        sys.exit(f"result {label!r} has no field {field!r}")
    sys.stdout.write(v if isinstance(v, str) else json.dumps(v, indent=2))


def cmd_export(run, out):
    d, recs = load(run)
    state = {
        "workflow": os.path.basename(d),
        "exported_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "done": {}, "failed": [], "unfinished": [],
    }
    for r in recs:
        if r["state"] == "done":
            state["done"].setdefault(r["label"], []).append(r["result"])
        elif r["state"] == "running":
            state["unfinished"].append({"label": r["label"], "phase": r["phase"]})
        else:
            state["failed"].append({"label": r["label"], "phase": r["phase"], "state": r["state"]})
    with open(out, "w") as f:
        json.dump(state, f, indent=1)
    print(f"{out}: {len(state['done'])} done labels, {len(state['failed'])} failed, "
          f"{len(state['unfinished'])} unfinished")


def main(a):
    if len(a) >= 2 and a[0] == "status":
        cmd_status(a[1:])
    elif len(a) >= 2 and a[0] == "results":
        prefix = a[a.index("--label") + 1] if "--label" in a else ""
        cmd_results(a[1], prefix)
    elif len(a) == 4 and a[0] == "field":
        cmd_field(a[1], a[2], a[3])
    elif len(a) == 3 and a[0] == "export":
        cmd_export(a[1], a[2])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
