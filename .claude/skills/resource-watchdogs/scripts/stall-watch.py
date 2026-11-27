#!/usr/bin/env python3
"""Read-only stall watch for Claude Code workflow agents.

A workflow agent whose tool result never came back looks exactly like a slow one: the workflow
only reports at the end. This watch reads each run's journal.jsonl, finds agents that started
and have no result yet, and measures how long each agent's transcript has been idle. It prints
one STALL line per agent idle longer than --idle-min (with the last tool call it made) and exits
5, so a background task notification wakes the orchestrator. It never stops or messages anything.

Where the files are: <session dir>/subagents/workflows/<run id>/journal.jsonl and
agent-<agent id>.jsonl beside it, where <session dir> is
~/.claude/projects/<project slug>/<session id>.

usage:
  python3 -I stall-watch.py --session-dir DIR [--run RUN_ID ...] [--idle-min 25]
                            [--interval 120] [--once] [--keep-running] [--exit-when-done]

Pass --run for the runs you know are live. A run stopped on purpose (TaskStop) leaves agents
that started and never finished, which this watch cannot tell from a hang; it marks a run whose
journal is as quiet as its agents.

Exit codes: 0 done or --once with no stall; 2 usage error; 5 stall found.
"""
import argparse
import glob
import json
import os
import sys
import time

EXIT_STALL = 5


def journal_state(run_dir):
    """{key: started entry} for agents that started and have no result, failure or skip yet."""
    path = os.path.join(run_dir, "journal.jsonl")
    if not os.path.exists(path):
        return None
    started, done = {}, set()
    with open(path) as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                continue
            kind, key = e.get("type"), e.get("key")
            if kind == "started" and key:
                started[key] = e
            elif kind in ("result", "failed", "skipped") and key:
                done.add(key)
    return {k: v for k, v in started.items() if k not in done}


def last_tool_call(transcript, tail_bytes=262144):
    """(tool name, short input, waiting for its result?) from the end of a transcript."""
    try:
        size = os.path.getsize(transcript)
        with open(transcript, "rb") as f:
            f.seek(max(0, size - tail_bytes))
            lines = f.read().decode("utf-8", "replace").splitlines()
    except OSError:
        return None, "", False
    answered = set()
    for line in reversed(lines):
        try:
            e = json.loads(line)
        except ValueError:
            continue
        content = (e.get("message") or {}).get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_result":
                answered.add(block.get("tool_use_id"))
            elif block.get("type") == "tool_use":
                args = block.get("input") or {}
                text = args.get("command") if isinstance(args.get("command"), str) else json.dumps(args)
                text = " ".join(text.split())[:160]
                return block.get("name"), text, block.get("id") not in answered
    return None, "", False


def runs_to_watch(workflows_dir, wanted, recent_hours):
    if wanted:
        return [os.path.join(workflows_dir, r) for r in wanted]
    cutoff = time.time() - recent_hours * 3600
    return sorted(d for d in glob.glob(os.path.join(workflows_dir, "*"))
                  if os.path.exists(os.path.join(d, "journal.jsonl"))
                  and os.path.getmtime(os.path.join(d, "journal.jsonl")) > cutoff)


def scan(runs, idle_min):
    now = time.time()
    rows, stalls, live = [], [], 0
    for run_dir in runs:
        running = journal_state(run_dir)
        if running is None:
            continue
        run = os.path.basename(run_dir)
        # A run stopped with TaskStop leaves started lines with no result. When the journal itself
        # is as quiet as the agents, say so: the run is more likely stopped than one agent hung.
        quiet = (now - os.path.getmtime(os.path.join(run_dir, "journal.jsonl"))) / 60 > idle_min
        for key, e in running.items():
            live += 1
            aid = e.get("agentId") or ""
            transcript = os.path.join(run_dir, f"agent-{aid}.jsonl")
            if not aid or not os.path.exists(transcript):
                continue
            idle = (now - os.path.getmtime(transcript)) / 60
            label = e.get("label") or key[:16]
            rows.append((run, label, idle))
            if idle > idle_min:
                name, text, waiting = last_tool_call(transcript)
                state = "waiting for its result" if waiting else "answered"
                note = " (journal quiet too: run stopped or wedged?)" if quiet else ""
                stalls.append((run, key, f"STALL {run} {label}: transcript idle {idle:.0f} min{note}; last tool "
                                         f"{name or 'none'} ({state}): {text}; {transcript}"))
    return rows, stalls, live


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--session-dir", required=True, help="~/.claude/projects/<slug>/<session id>")
    p.add_argument("--run", action="append", default=[], help="workflow run id (repeatable; default: recent runs)")
    p.add_argument("--recent-hours", type=float, default=24, help="without --run: runs whose journal changed lately")
    p.add_argument("--idle-min", type=float, default=25, help="transcript idle minutes that count as a stall")
    p.add_argument("--interval", type=float, default=120, help="seconds between scans")
    p.add_argument("--once", action="store_true", help="print running agents and their idle time, then exit")
    p.add_argument("--keep-running", action="store_true", help="print stalls and keep watching")
    p.add_argument("--exit-when-done", action="store_true", help="exit 0 once no watched agent is running")
    a = p.parse_args(argv)
    workflows = os.path.join(os.path.expanduser(a.session_dir), "subagents", "workflows")
    if not os.path.isdir(workflows):
        print(f"no workflows folder at {workflows}")
        return 2
    alerted = set()
    while True:
        runs = runs_to_watch(workflows, a.run, a.recent_hours)
        rows, stalls, live = scan(runs, a.idle_min)
        if a.once:
            for run, label, idle in sorted(rows, key=lambda r: -r[2]):
                print(f"{run}  {label}  idle {idle:.0f} min")
            for _, _, line in stalls:
                print(line)
            return EXIT_STALL if stalls else 0
        fresh = [s for s in stalls if (s[0], s[1]) not in alerted]
        for run, key, line in fresh:
            alerted.add((run, key))
            print(line, flush=True)
        if fresh and not a.keep_running:
            return EXIT_STALL
        if a.exit_when_done and live == 0:
            print(f"DONE: no running agents in {len(runs)} watched run(s)", flush=True)
            return 0
        time.sleep(a.interval)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
