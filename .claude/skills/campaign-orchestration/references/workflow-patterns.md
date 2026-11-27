# Workflow patterns that worked

Skeletons of the Workflow scripts that carried the 2026 campaigns. They use the Workflow tool's
script API (`meta`, `agent(prompt, opts)`, `parallel(thunks)`, `pipeline(items, ...stages)`,
`phase(title)`, `log(msg)`); load the `workflow-authoring` skill for the full API. `COMMON`, the
schemas and the brief wording are in [agent-briefs.md](agent-briefs.md).

Every pattern shares four habits:

- Every `agent()` call has a `label`, a `phase` and a `schema`. Labels are how you read the journal.
- Model by role: no `model` (Opus, the session model) for implementation, research and synthesis;
  `model: 'sonnet'` for review, verification and tests.
- Filter `null` results (`.filter(Boolean)`) and return early with a `stopped:` reason when a
  required step fails, so the orchestrator sees why.
- Large inputs are sliced before they go into a prompt (`JSON.stringify(x).slice(0, N)`), and agents
  are pointed at files on disk for the full text.

## Scout, deep readers, report per source (pipeline)

Use for research over many sources (repos, docs, catalogs). `pipeline()` lets each source move
through its stages without waiting for the slowest source; only the synthesis waits for all.

```js
const perSource = pipeline(SOURCES,
  (x) => agent(`${COMMON}\nScout ${x.name}: clone (depth 1) into a fresh empty dir, read the
licence, inventory every item, split it into at most ${MAX_CHUNKS} chunks with exact paths.
Write ${OUT}/${slug(x)}/inventory.md. Return the schema.`,
    { label: `scout ${x.name}`, phase: 'Scout', schema: SCOUT }),
  (scout, x) => parallel((scout.chunks || []).slice(0, MAX_CHUNKS).map((c, i) => () =>
    agent(`${COMMON}\nDeep-read chunk ${i + 1} of ${x.name}: ${JSON.stringify(c.paths)}.
For each item give a verdict against our catalog. Write ${OUT}/${slug(x)}/chunk-${i + 1}.md.
Return the schema.`, { label: `read ${x.name} ${i + 1}`, phase: 'Deep read', schema: READ })))
    .then(reads => ({ scout, reads: reads.filter(Boolean) })),
  (res, x) => agent(`${COMMON}\nWrite the report for ${x.name} from these reads:
${JSON.stringify(res.reads).slice(0, 60000)}. Put the full markdown in report_md.`,
    { label: `report ${x.name}`, phase: 'Report', schema: REPORT }),
)
const reports = (await perSource).filter(Boolean)
```

A security review per adoption candidate slots in as one more stage before the report (see
the `external-skill-triage` skill for the checklist).

## Synthesis plus critics

One synthesizer, then critics in parallel, then a final agent that accepts or rejects every
critique with one line of reasoning written to a file. The critics catch what the synthesis
dropped; the written decisions stop the final agent from quietly ignoring them.

```js
const draft = await agent(`${COMMON}\nMerge these reports into one plan ...`,
  { label: 'synthesize', phase: 'Plan', schema: PLAN })
const critiques = (await parallel([
  () => agent(`${COMMON}\nCompleteness critic: what did the plan drop? ...`,
    { label: 'critic completeness', phase: 'Plan', schema: FINDINGS }),
  () => agent(`${COMMON}\nValue and licence critic: bloat, duplicates, licence problems ...`,
    { label: 'critic value', phase: 'Plan', schema: FINDINGS }),
])).filter(Boolean)
const final = await agent(`${COMMON}\nAccept or reject each critique finding with one line of
reasoning in ${OUT}/critique-decisions.md, then write the final plan.
Critiques: ${JSON.stringify(critiques).slice(0, 30000)}`,
  { label: 'final plan', phase: 'Plan', schema: PLAN })
```

## Investigation, design, lanes, integrator

Use for a multi-part code change in one repo. Investigators (Opus, one per question) report; one
design agent writes `DESIGN.md` with contracts, waves and lanes, each lane owning an exact file
list. Lanes run in parallel in ONE worktree and never commit; one integrator per wave runs the
gates, commits each lane separately and pushes. The next wave reads the committed code.

```js
const lane = (l, ph) => agent(`${COMMON}
Other lanes are editing other files in this worktree right now. Edit ONLY these files:
${JSON.stringify(l.owns_files)}. Other lanes own: ${JSON.stringify(othersFiles(l))}.
If you need a change in a file you do not own, put it in your handoff field. Do NOT commit,
stash, reset or checkout. Run cargo commands one at a time. A build error in a file you do not
own is another lane's edit in progress: wait and re-run, do not fix it.
Lane ${l.lane}: ${l.goal}. Tests: ${JSON.stringify(l.tests)}. Return the schema.`,
  { label: `lane ${l.lane}`, phase: ph, schema: LANE })

const integrate = (results, wave) => agent(`${COMMON}
You are the wave ${wave} integrator and the only committer. Lane results:
${JSON.stringify(results).slice(0, 30000)}
1. Run every lane's gates plus the repo gates (see the pr-ci-merge skill). Fix real breaks in
   the owning lane's files; list failures a later lane resolves.
2. Commit each lane separately with git add of exactly that lane's files.
3. git fetch origin; merge origin/main if it moved; re-run the quick gates; push the branch.
Put your full report in the summary. Return the schema.`,
  { label: `integrate wave ${wave}`, phase: `Integrate ${wave}`, schema: LANE })

for (const [i, wave] of WAVES.entries()) {
  phase(`Wave ${i + 1}`)
  const r = (await parallel(wave.map(l => () => lane(l, `Wave ${i + 1}`)))).filter(Boolean)
  log(`wave ${i + 1}: ${r.map(x => x.lane + '=' + x.status).join(', ')}`)
  const g = await integrate(r, i + 1)
  if (!g || g.status === 'blocked') return { stopped: `wave ${i + 1} integration blocked`, r, g }
}
```

Rules that make lanes safe:

- File ownership is exact. Name the other lanes' files in every lane brief; "touch nothing else"
  is not enough.
- A signature change that another lane's file calls cannot run in parallel with that lane. Put it
  in an earlier wave.
- Generated files (the OpenAPI spec and TS client) belong to one lane, regenerated by the
  integrator after route changes.
- The integrator commits per lane so each finding stays revertible; no AI trailer (pr-ci-merge).

## Fix-and-review loop with a round cap and a release flag

Use for anything security sensitive (a de-identifier, a guard, a gate). The fixer and the
releaser are different agents; reviewers are independent, each with its own lens, and judge by a
fixed severity rubric pasted into every prompt.

```js
const RELEASE_AFTER_CLEAN = false   // the owner flips this; publishing is the owner's word
const MAX_ROUNDS = 6                // set before the loop; at the cap, report residuals
let findings = [], history = [], clean = false, fix = null, verdicts = []
for (let round = 1; round <= MAX_ROUNDS && !clean; round++) {
  fix = await agent(FIX(findings, round), { label: `fix r${round}`, phase: 'Fix', schema: RESULT })
  if (!fix || fix.status !== 'done') return { stopped: 'fix failed', round, fix }
  verdicts = (await parallel(LENSES.map((lens, i) => () => agent(
    REVIEW(fix, round, history.concat(findings), lens),
    { label: `review r${round} ${'ABC'[i]}`, phase: 'Review', model: 'sonnet', schema: VERDICT }))))
    .filter(Boolean)
  history.push(...findings)
  const open = verdicts.flatMap(v => v.findings || [])
    .filter(f => f.severity === 'high' || f.severity === 'medium')
  log(`round ${round}: ${verdicts.length}/${LENSES.length} reviews, ${open.length} high/medium`)
  if (verdicts.length === LENSES.length && open.length === 0) clean = true
  else findings = verdicts.flatMap(v => v.findings || []).filter(f => f.severity !== 'info')
}
if (!clean) return { stopped: `high/medium left after ${MAX_ROUNDS} rounds`, fix, verdicts }
if (!RELEASE_AFTER_CLEAN) return { clean: true, held_for_owner: true, fix, verdicts }
const rel = await agent(RELEASE_BRIEF, { label: 'release', phase: 'Release', schema: RESULT })
// then two Sonnet reviews of the PUBLISHED bytes (download into a fresh dir, check the digest)
// then, only if both are clean, the agent that updates the marketplace card or registry
```

Stop rules, decided before the loop:

- Clean means no high and no medium, from every lens that returned. A missing review is not clean.
- For lab test passes the owner's bar is "no new medium or higher finding that hurts the user
  experience"; lows go on a found-not-fixed list; one clean focused pass is enough to ship.
- An open input space (names, dates, file shapes) never reaches zero mediums by patching. After
  two or three rounds that each find a narrower leak, redesign to fail closed (keep only what can
  be positively verified) instead of adding heuristics. Eleven rounds cost about 14 points of
  weekly usage before the redesign changed the trend.
- At the cap, bring the residual list to the owner. Do not loop past it.
- Reviewers of a fix get every earlier finding and are told to look for new ways in that the fix
  opened, with generated variety (dozens of header or input variants, not only the listed ones).
- Separate wording mismatches (docs promise more than the code does) from real leaks, and fix the
  wording in the same round.

## Test pass with adversarial verification

Use for a live test pass of the app. Testers (Sonnet), one per chunk of the feature inventory,
each in its own slot (run name, CDP port, serve ports, sandbox folder), report PASS, FAIL,
EXPLAINED (not drivable here, with the covering test run) or BLOCKED, plus findings. Every FAIL
and finding then gets three independent tasks:

- REFUTE: try to show it is by design, tester error, a stale contract, the environment or already
  fixed. Default to "not real" when evidence is weak.
- SEVERITY: judge impact and name the owner rule it breaks.
- REPRODUCE: re-run it in a fresh slot. Needed for every FAIL and whenever the first two disagree.

A finding is confirmed by at least 2 of 3 real votes. A read-only completeness critic then lists
what was not exercised (surfaces, OSes, theme family and mode, privacy tiers). Pass the testers a
KNOWN list (already confirmed, rejected or fixed) so they do not re-report it.

## Fix wave from confirmed findings

Plan (Opus): for each finding decide fix, no_change (cite the decision record) or owner_decision,
and partition fixes into 6 to 10 file-disjoint lanes with root cause at file:line, approach and a
failing-first test. Implement lanes (Opus, parallel), review each lane (Sonnet, read-only, runs
the lane's tests; one fix-up and re-review if needed), run the gates one heavy command at a time
(Sonnet; re-run a failure alone before counting it; up to three gate-fix rounds), vision-check UI
lanes in light and dark across all three theme families, then commit per lane and open the PR.

## Pipelines over barriers

- `pipeline()` for per-item chains; `await parallel()` only where the next step needs every
  result (synthesis, an integrator).
- Two independent streams in one script: `await parallel([() => streamA, () => streamB])`.
- Keep slot-dependent or time-dependent text out of prompts you may want to re-run from the cache:
  any change to a prompt changes its cache key.
