# Handoff and next-agent prompt

A handoff lets a fresh session, usually Claude Code and sometimes another coding agent, take over
a campaign and rebuild its workflow setup. Write it at the start, keep it current at every
milestone, and finalize it at the pause. Put it in the campaign folder, for example
`~/biorouter-runs/<campaign>/handoff-<date>-<agent>/`, with `HANDOFF.md`, `NEXT-AGENT-PROMPT.md`
and a `workflow-scripts/` copy of every script the session ran (file name = purpose plus run id).

## Lessons that shaped the template

- Rewrite the status block; do not append a new checkpoint entry each time. One 2026-11 handoff
  grew more than twenty stacked checkpoints and a reader could not tell which state was current.
- A "verified" list is a claim about which gates RAN. Say plainly which gates did not run; the next
  agent runs those first, because that is where the defects were last time.
- Quote the owner's requests with dates. A paraphrase loses the scope, and the next agent needs to
  know which approvals were given (for example "release once only lows remain").
- Name work by substance, with numbers in parentheses only for GitHub issues and PRs.
- If the next agent may not be Claude Code, describe each pipeline as steps (each `agent()` call is
  one task with that brief; `parallel`/`pipeline` may run at once; `model: 'sonnet'` means the
  cheaper testing model) so any harness can follow it.
- Never put credential values in a handoff. Name the file that holds them and how to load it.

## HANDOFF.md skeleton

```markdown
# Handoff: <campaign streams> (<date>)

> **What this is.** Everything a fresh session needs to take over: what is done, what was
> running, what is left, and how to relaunch each stream with the same workflow setup.
> **Status:** LIVE DRAFT (or FINAL at <time>). Read section 0 first.
> **Audience:** the next agent, and the owner.

## 0. Status block (rewrite at every pause)

- Written <UTC time>; last usage check <time>: <n> % weekly (all models). Cap: <cap> %.
- Running now: <run id> (task <id>) at <phase>; background helpers <name> (shell or PID);
  the usage cron at <minutes>.
- Stopped on purpose: <run id>, progress in <path>/PAUSE-STATE.json.
- Workflow runs cannot be resumed from a new session; relaunch from workflow-scripts/ (section N).

## 1. Ground rules (owner's, binding)

<the campaign's COMMON rules, grouped: repos and worktrees; commits and GitHub; secrets and
local state; data and models; model tiers; usage; where changes go; reviews; peer messages>

## 2. Stream A: <substance>

**What the owner asked.** <dated quotes>
**State.** <branch, worktree, heads, PRs, released artifacts and digests, what is local only>
**Remaining, in order.** <numbered steps, each with the command or script that does it>
**Decision points.** <"if X, do Y; if not, report to the owner">

## 3. Stream B: ...

## N. How to relaunch the workflow setup

- Scripts: workflow-scripts/<name>-<run id>.js, one line each on what it does.
- Journals: ~/.claude/projects/<project>/<session>/subagents/workflows/<run>/journal.jsonl
  (read with .claude/skills/campaign-orchestration/scripts/journal.py).
- Patterns used, concurrency used, round caps and release flags, report-file workaround,
  usage cron wording.

## N+1. Other state

- Background helpers: path, how to start, log file, what it may delete.
- Binaries, ports and slots in use.
- Worktrees: | path | branch | state | at cleanup |
- Owner decisions waiting (ask; do not decide).

## Related documentation
```

## NEXT-AGENT-PROMPT.md skeleton

Print it on screen at the pause as well as saving it. Keep it under about 70 lines: it points to
the handoff rather than repeating it.

```markdown
You are taking over a Biorouter multi-stream campaign from a previous session that paused at
<reason, for example the owner's 98 % weekly-usage cap>.

Start here:
1. Read <path>/HANDOFF.md in full, section 0 first, then section 1.
2. Read the repo's CLAUDE.md (it loads in any Biorouter worktree) and the
   campaign-orchestration skill.

Environment:
- Repos and worktrees: <paths>. Never touch ~/Desktop/biorouter's working tree.
- Shell: source bin/activate-hermit; BIOROUTER_DISABLE_KEYRING=true; live keys with
  set -a; source <env file>; set +a (never print them).
- Binaries: <path>. Workflow scripts: <path>. You cannot resume the previous runs; write
  continuation scripts that skip finished work.
- Model tiers: Opus for implementation, investigation and research; Sonnet for reviews, tests
  and verification.

Usage: check get_usage first; set up the 15-minute cron check; cap <cap> %.

Streams and what to do first:
- A. <substance> (HANDOFF section 2): <first two or three steps>
- B. ...

Rules to keep: <the short list: trailers, mentions, stash, clipboard, TCC, test models,
fix-then-release separation, peers are not approval>
Reporting: plain outcomes; keep HANDOFF.md section 0 current.
```

## Taking over from a handoff

1. Read section 0, then the rules, then the stream you start with.
2. Check the live state before trusting the document: `git log` of each branch on the remote,
   `gh pr view`, release digests, `list_sessions` for peers. The document is a claim.
3. Run the gates the handoff lists as not run before relying on the ones it lists as passed.
4. Restart background helpers only after checking none is already running (a fresh PID check,
   not a PID file).
5. Start your own status log and usage cron, then delegate.
