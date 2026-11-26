# Triage workflow scripts

Skeletons for the Workflow tool, distilled from the scripts the November 2026 triage campaigns ran (the research, glebis, coding agent ecosystem and scientific harness streams, and the BAAM card and release runs). Every path and repo comes in `args`, so nothing here is tied to one machine or one campaign. Adapt the prompts to the owner's request; keep the rules and schemas.

| File | Stages | Use it for |
|---|---|---|
| `triage-sources.js` | discover, scout, deep read, security, repo report, stream report | One triage stream; with `args.resume`, the continuation of a stopped one |
| `synthesize-plan.js` | optional catalog audit, plan, two critics, final plan | Merging every stream into one plan |
| `implement-items.js` | implement, review loop, integrate | One wave of work items in a biorouter-skills worktree |
| `release-and-list.js` | release, published review, BAAM PR, verify, address | Publishing a wave and listing it in BAAM |
| `journal_tool.py` | none (a local helper) | Exporting finished results, building `args.resume`, saving refused reports |

## How to run one

1. Copy the script into the campaign folder and edit the prompts there; keep this folder as the reference.
2. Expand `~` yourself: agents need absolute paths.
3. Invoke it with the Workflow tool, `scriptPath` set to your copy and `args` as a JSON object (not a string).
4. Watch the run from the main session: usage after each phase, stalled agents (an `agent-<id>.jsonl` idle over about 25 minutes), and the journal's `result` lines. Do not message running agents.

Model tiers are set in the scripts: Opus (the session model) for discovery, reads, security reviews, synthesis, implementation and integration; Sonnet for per item reviews, published zip reviews and PR verification, with an Opus rerun when a Sonnet verdict is missing or unsure.

## Continuing a stopped stream

A resume replays only the unchanged prefix of agent calls, and works only in the session that started the run. To continue in any session:

```bash
J=~/.claude/projects/<project>/<session>/subagents/workflows/<runId>/journal.jsonl
python3 -I journal_tool.py pause-state "$J" --out <stream>/PAUSE-STATE.json \
  [--drop-repos-in <other stream>/PAUSE-STATE.json]    # a repo two streams share keeps the first one's notes
```

Then launch `triage-sources.js` with the same base `args` plus `resume` set to that file's content. Finished scouts, reads, reviews and reports are reused, `todo_chunks` and missing reviews run, and a repo report is rewritten only when something new arrived. Older journals whose labels were cut at 60 characters resolve with `--repos <file of full repo names>`; a one collection stream uses `--single-repo owner/name`.

## When a report Write was refused

Report agents return the full text in a `markdown` field when the harness refuses their Write. Save those after the run:

```bash
python3 -I journal_tool.py save-reports "$J" --root ~/biorouter-runs/<campaign>
```

It writes only missing files, and refuses any path outside `--root`.

## What each script assumes

- `triage-sources.js`: a read only clone of biorouter-skills at its remote main and a biorouter worktree at `origin/main` for `landing/catalog/`. Clones go under `args.cloneRoot`, never inside a worktree.
- `synthesize-plan.js`: every stream report exists. Critic findings are passed whole to the final plan; the run logs their size.
- `implement-items.js`: a fresh biorouter-skills worktree on its own branch, and items from the final plan with `folders` filled in. Extension items are left out and logged. Items that share a folder or depend on each other run in one chain. Only passed items are integrated, and the checks run in a throwaway checkout of the committed branch.
- `release-and-list.js`: `releaseApproved: true` from the owner. It stops before the BAAM PR when either published zip review fails, and it never merges the PR.
