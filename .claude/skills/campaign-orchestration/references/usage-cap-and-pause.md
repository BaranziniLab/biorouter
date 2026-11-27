# Usage cap, organic wind-down and continuation

The owner runs long campaigns on a weekly usage plan. Heavy Opus workflows spend it fast: five at
once burned about 30 % of the weekly window per hour on 2026-11-26, and a hard stop at 100 % would
strand uncommitted work that no other session can resume. So the session watches the meter,
winds down at phase boundaries, and leaves a handoff that lets a fresh session rebuild the setup.

## Reading the meter

- Tool: `mcp__ccd_session_mgmt__get_usage` (deferred; load it with ToolSearch `select:` first).
- The binding number is "Weekly · all models", not the session's context window.
- The owner sets the cap per campaign. 98 % with an organic wind-down is the latest (2026-11-26);
  80, 88 and 90 % have been used for single runs. If the brief names none, ask. If the owner says
  they watch usage themselves (as in the 2026-11-12 merge campaign), do not monitor or pause.
- Check before every expensive launch and after every workflow finishes, as well as on the timer.

## The timer

Create a recurring CronCreate job every 15 minutes whose prompt is self-contained, so it still
makes sense when it fires into a busy session. The wording that worked:

```text
Usage cap check (owner rule, <CAP> % weekly cap, set <DATE>): call
mcp__ccd_session_mgmt__get_usage and read "Weekly · all models". Under 90 %: note it in one line in
<STATUS FILE> and carry on. At 90 % or more: launch nothing new and let near-finished workflows
finish. At 95 % or more: stop every running workflow with TaskStop, export the journals' progress
to disk, finalize the handoff at <HANDOFF.md> and <NEXT-AGENT-PROMPT.md>, print the agent prompt on
screen, then stop working and delete this cron job.
```

Adjust the 90 and 95 lines to the owner's cap (keep a few points of margin below it: one more
workflow phase can cost several points).

## Organic wind-down

The owner asked that a pause not cut jobs off abruptly. In order:

1. From the "launch nothing new" line: no new workflows, lanes or test suites. Let agents that are
   close to done finish, because their result is cached in the journal and on disk.
2. Stop the biggest fan-outs first, at a phase boundary (after a wave integrates, after a batch of
   reads lands), not in the middle of an implementer's edit.
3. At the stop line: `TaskStop` each remaining workflow task. Stop dev apps, serve daemons and tmux
   sessions you started, by PID or through their launcher.
4. Export each stopped run's progress:
   `python3 -I .claude/skills/campaign-orchestration/scripts/journal.py export <run> <folder>/PAUSE-STATE.json`.
   For research fan-outs, also record which scouts and which chunk numbers are unfinished per
   source; the 2026-11-26 triage files used the keys `discovery`, `scouts`, `reads_done` and
   `reads_unfinished`.
5. Snapshot uncommitted work in each worktree: `git diff --binary > <pause folder>/wip-<branch>.patch`
   and a tar of untracked files. Commit finished lanes locally, one commit per lane. Do not push
   half-done work.
6. Rewrite the handoff's status block, list every running and stopped run with its PAUSE-STATE
   path, update the next-agent prompt, print the prompt on screen, give the owner both paths, and
   delete the cron job.

Keep the handoff current from the first milestone so step 6 is an edit, not a rewrite.

## Resume is same-session only

- `resumeFromRunId` works only in the session that launched the run. A new session cannot resume
  it: the handoff must say how to relaunch.
- Even in the same session, a resume replays only the longest unchanged PREFIX of `agent()` calls.
  The first call with no stored result (an agent still running at the pause) and every call after
  it run live again, finished ones included. A paused fan-out always has such a call early, so a
  resume is effectively a fresh run.
- Editing a prompt changes its cache key, and so does any prompt that embeds an earlier result
  that does not reproduce byte for byte. After any resume, tail the journal for new `started` lines
  on stages that should have been cached, and stop it at once if they appear.

## Continuation scripts

To continue paused work, write a new script that runs only what is left:

- Read the PAUSE-STATE file (or the journal) for finished results, then pass what is left to the
  new run as its `args` (the 2026-11-16 pass continuation took `{pass, sha, known, chunks,
  pending, seen}`), or embed it in the script as constants.
- Skip any phase whose outputs already exist on disk; tell agents to read finished outputs from
  their files rather than pasting them all into prompts.
- Dedupe items that two paused runs share, keeping the one with more finished work.

```js
// args: { scouts: {repo: scoutResult}, unfinished: [{repo, n}] }, built from PAUSE-STATE.json
const reads = (await parallel(args.unfinished.map(t => () => agent(`${COMMON}
Continue the deep read of ${t.repo}, chunk ${t.n}; the scout result is
${JSON.stringify(args.scouts[t.repo]).slice(0, 6000)}. Earlier chunks' notes are in
${OUT}/${slug(t.repo)}/ ...`,
  { label: `read ${t.repo} ${t.n}`, phase: 'Deep read', schema: READ })))).filter(Boolean)
```

The same method switches the remaining agents of a running workflow to a cheaper model: stop it,
compute the unfinished items, and launch them in a new small workflow. Editing the model in the
script and resuming re-runs finished work.

## Related

- [handoff-template.md](handoff-template.md) for the handoff and the next-agent prompt.
- The `resource-watchdogs` skill for disk, memory and stall watchers to restart in a new session.
