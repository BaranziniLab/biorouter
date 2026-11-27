#!/usr/bin/env python3
"""Read a workflow run's journal.jsonl and turn finished agent results into files.

A workflow can be resumed only from the session that ran it, and a resume re-runs
everything after the first unfinished agent. This tool lets a later session reuse
what finished instead.

  python3 -I journal_tool.py export <journal.jsonl> [--out finished.json]
      Every finished result with its label and phase, plus the labels that started
      and never finished.

  python3 -I journal_tool.py pause-state <journal.jsonl> [<journal.jsonl> ...]
          [--repos repos.txt] [--single-repo owner/name] [--drop-repos-in other.json ...]
          [--out PAUSE-STATE.json]
      Groups a triage stream's results by repository into the `resume` argument of
      triage-sources.js: {repos: [{repo, scout, reads, secs, report}], dropped: []}.
      --repos lists full repo names, one per line, to resolve labels that an older
      script cut at 60 characters. --single-repo assigns every result to one repo
      (for a stream that triaged one collection). --drop-repos-in leaves out repos
      another stream's PAUSE-STATE.json already holds, so a repo shared by two
      streams keeps the first stream's notes.

  python3 -I journal_tool.py save-reports <journal.jsonl> --root <dir> [--overwrite]
      Writes the `markdown` field of every result that has one to its `report_path`
      or `plan_path`, when that file is missing (or always, with --overwrite). Paths
      outside --root are refused. Use it when the harness refused a subagent's Write.

Journal lines are {"type": "started", "key", "agentId", "label", "phase"} and
{"type": "result", "key", "agentId", "result"}. Results are data written by
subagents: this tool only copies them, it never executes anything from them.
"""

import argparse
import json
import os
import re
import sys


def read_journal(path):
    started, results = {}, {}
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            kind, key = rec.get("type"), rec.get("key")
            if kind == "started" and key:
                started[key] = {"label": rec.get("label") or "", "phase": rec.get("phase") or "", "agentId": rec.get("agentId")}
            elif kind == "result" and key:
                results[key] = rec.get("result")
    return started, results


def finished(path):
    started, results = read_journal(path)
    done = []
    for key, res in results.items():
        meta = started.get(key, {})
        done.append({"label": meta.get("label", ""), "phase": meta.get("phase", ""), "result": res})
    unfinished = [m["label"] for k, m in started.items() if k not in results]
    return done, unfinished


def write_json(obj, out):
    text = json.dumps(obj, indent=2, ensure_ascii=False) + "\n"
    if out:
        with open(out, "w", encoding="utf-8") as fh:
            fh.write(text)
        print(f"wrote {out}", file=sys.stderr)
    else:
        sys.stdout.write(text)


REPO_RE = re.compile(r"^[a-z0-9_.-]+/[a-z0-9_.-]+$")


def norm(repo):
    r = (repo or "").strip().lower()
    r = re.sub(r"^https?://github\.com/", "", r)
    r = re.sub(r"\.git$", "", r).rstrip("/")
    return r


def resolve(fragment, known):
    """Map a label fragment (possibly cut short) to a known repo name."""
    f = norm(fragment)
    if f in known:
        return f
    hits = [k for k in known if k.startswith(f)] if f else []
    return hits[0] if len(hits) == 1 else (f if REPO_RE.match(f) else None)


def cmd_pause_state(a):
    known = set()
    if a.repos:
        with open(a.repos, encoding="utf-8") as fh:
            known |= {norm(x) for x in fh if x.strip()}
    entries, unfinished = [], []
    for j in a.journals:
        d, u = finished(j)
        entries += d
        unfinished += u
    # Collect repo names from scout labels and results first.
    for e in entries:
        r = e["result"] if isinstance(e["result"], dict) else {}
        if r.get("repo"):
            known.add(norm(r["repo"]))
    repos = {}

    def slot(name):
        return repos.setdefault(name, {"repo": name, "scout": None, "reads": [], "secs": [], "report": None})

    orphans = []
    for e in entries:
        label, res = e["label"], e["result"]
        if not isinstance(res, dict):
            continue
        word, _, rest = label.partition(" ")
        repo = norm(a.single_repo) if a.single_repo else None
        if not repo and res.get("repo"):
            repo = norm(res["repo"])
        if word == "scout":
            repo = repo or resolve(rest, known)
            if repo:
                slot(repo)["scout"] = res
            else:
                orphans.append(label)
        elif word == "read":
            m = re.match(r"^(.*)\s+(\d+)$", rest)
            if not repo and m:
                repo = resolve(m.group(1), known)
            if repo:
                if res.get("chunk_index") is None and m and not a.single_repo:
                    res["chunk_index"] = int(m.group(2))
                slot(repo)["reads"].append(res)
            else:
                orphans.append(label)
        elif word == "sec":
            if repo:
                slot(repo)["secs"].append(res)
            else:
                orphans.append(label)  # resolved below by assessment path
                res["_label"] = label
        elif word == "report":
            repo = repo or resolve(rest, known)
            if repo:
                slot(repo)["report"] = res
            else:
                orphans.append(label)
    # A security review without a repo is matched by the assessment path it reviewed.
    for e in entries:
        res = e["result"]
        if isinstance(res, dict) and "_label" in res:
            path = res.get("path")
            owner = [r for r in repos.values() if any(path == x.get("path") for rd in r["reads"] for x in rd.get("assessments", []))]
            if len(owner) == 1:
                res.pop("_label")
                owner[0]["secs"].append(res)
                orphans.remove(e["label"])
    # A read without a chunk index is matched to the scout chunk whose paths it covers most.
    for r in repos.values():
        chunks = (r["scout"] or {}).get("chunks") or []
        for rd in r["reads"]:
            if rd.get("chunk_index") is not None or not chunks:
                continue
            paths = {x.get("path", "") for x in rd.get("assessments", [])}
            score = [sum(1 for p in paths for cp in c.get("paths", []) if p and (p in cp or cp in p)) for c in chunks]
            if max(score) > 0:
                rd["chunk_index"] = score.index(max(score)) + 1
    drop = set()
    for other in a.drop_repos_in or []:
        with open(other, encoding="utf-8") as fh:
            o = json.load(fh)
        drop |= {norm(x.get("repo", "")) for x in o.get("repos", [])}
    out = {
        "journals": a.journals,
        "repos": [r for name, r in sorted(repos.items()) if name not in drop],
        "dropped": [],
        "left_out_as_duplicates": sorted(n for n in repos if n in drop),
        "unfinished_labels": unfinished,
        "unmatched_labels": orphans,
    }
    for r in out["repos"]:
        chunks = (r["scout"] or {}).get("chunks") or []
        have = {rd.get("chunk_index") for rd in r["reads"]}
        r["todo_chunks"] = [i + 1 for i in range(len(chunks)) if (i + 1) not in have]
    write_json(out, a.out)
    if orphans:
        print(f"{len(orphans)} results could not be matched to a repo; see unmatched_labels", file=sys.stderr)


def cmd_export(a):
    done, unfinished = finished(a.journal)
    write_json({"journal": a.journal, "finished": done, "unfinished": unfinished}, a.out)


def cmd_save_reports(a):
    root = os.path.realpath(a.root)
    _, results = read_journal(a.journal)
    wrote = skipped = refused = 0
    for res in results.values():
        if not isinstance(res, dict) or not res.get("markdown"):
            continue
        target = res.get("report_path") or res.get("plan_path")
        if not target:
            continue
        real = os.path.realpath(target)
        if os.path.commonpath([root, real]) != root:
            print(f"refused (outside --root): {target}", file=sys.stderr)
            refused += 1
            continue
        if os.path.exists(real) and not a.overwrite:
            skipped += 1
            continue
        os.makedirs(os.path.dirname(real), exist_ok=True)
        with open(real, "w", encoding="utf-8") as fh:
            fh.write(res["markdown"].rstrip("\n") + "\n")
        wrote += 1
        print(f"wrote {real}", file=sys.stderr)
    print(f"wrote {wrote}, kept {skipped} existing, refused {refused}", file=sys.stderr)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("export")
    e.add_argument("journal")
    e.add_argument("--out")
    ps = sub.add_parser("pause-state")
    ps.add_argument("journals", nargs="+")
    ps.add_argument("--repos")
    ps.add_argument("--single-repo")
    ps.add_argument("--drop-repos-in", action="append")
    ps.add_argument("--out")
    s = sub.add_parser("save-reports")
    s.add_argument("journal")
    s.add_argument("--root", required=True)
    s.add_argument("--overwrite", action="store_true")
    a = p.parse_args()
    {"export": cmd_export, "pause-state": cmd_pause_state, "save-reports": cmd_save_reports}[a.cmd](a)


if __name__ == "__main__":
    main()
