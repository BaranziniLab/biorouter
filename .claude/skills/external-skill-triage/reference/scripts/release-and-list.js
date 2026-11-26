export const meta = {
  name: 'external-skill-triage-release',
  description: 'Release integrated skills from biorouter-skills, review the published zips independently, then open and verify the BAAM pull request',
  whenToUse: 'After implement-items.js integrated and committed a wave. Release runs only when args.releaseApproved is true.',
  phases: [
    { title: 'Release', detail: 'push main, release.mjs and verify-release per id, digest commit' },
    { title: 'Published review', detail: 'two independent reviews of the published zips', model: 'sonnet' },
    { title: 'BAAM PR', detail: 'sync the mirror, cards, registry, checks, pull request' },
    { title: 'Verify', detail: 'check the PR against primary sources', model: 'sonnet' },
    { title: 'Address', detail: 'fix findings, watch the required checks; never merge' },
  ],
}

// args (absolute paths):
// {
//   ids: ['skill-id', ...],                                  // the skills to release
//   skillsWt: '/abs/biorouter-runs/<campaign>/skills-wt',    // holds the integrated commits
//   work: '/abs/biorouter-runs/<campaign>/release-work',     // counts-before.tsv and rollback archives
//   biorouterBin: '/abs/<released biorouter CLI>',           // for release.mjs --biorouter install tests
//   biorouterRepo: '/abs/<a biorouter clone or worktree the BAAM worktree is added from>',
//   baamWt: '/abs/biorouter-runs/<campaign>/baam-wt', baamBranch: 'baam/<topic>-<yyyy-mm>',
//   jobsChanges: 'required jobs.json owner changes, if any',
//   releaseApproved: false,                                  // the owner flips this; without it nothing is published
// }

const A = args || {}
for (const k of ['ids', 'skillsWt', 'work', 'biorouterBin', 'biorouterRepo', 'baamWt', 'baamBranch']) if (!A[k]) throw new Error(`args.${k} is required`)
const COMMON = `
Rules: never touch ~/Desktop/biorouter or ~/Desktop/biorouter-skills working trees; never git stash, force push or rewrite pushed history; never delete a release or a tag; commits carry NO Co-Authored-By trailer (the required check no-ai-coauthor rejects it); no GitHub @mentions; never print credentials; never touch the clipboard; plain prose with no em dashes; never write the name of the project Biorouter was derived from or its publisher. Run long commands in the background with a log and poll them; stop every process you start. Set BIOROUTER_DISABLE_KEYRING=true. Your final message is data for an orchestrator.`
const RESULT = { type: 'object', properties: { status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
  items: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, version: { type: 'string' }, url: { type: 'string' }, sha256: { type: 'string' }, result: { type: 'string' } }, required: ['id', 'result'] } },
  commits: { type: 'array', items: { type: 'string' } }, pr_url: { type: 'string' },
  checks: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, exit_code: { type: 'integer' } }, required: ['command', 'exit_code'] } },
  summary: { type: 'string' } }, required: ['status', 'summary'] }
const VERDICT = { type: 'object', properties: { pass: { type: 'boolean' }, unsure: { type: 'boolean' },
  findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['high', 'medium', 'low', 'info'] }, id: { type: 'string' }, title: { type: 'string' }, evidence: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'title', 'evidence'] } } },
  required: ['pass', 'findings'] }

if (!A.releaseApproved) {
  log('releaseApproved is false: nothing is published. Ask the owner, then relaunch with releaseApproved: true.')
  return { held: true, ids: A.ids }
}

phase('Release')
const rel = await agent(`${COMMON}
You are the releaser. You do not fix findings: if anything fails, stop and report.
In ${A.skillsWt}: 1. git fetch the remote that points at BaranziniLab/biorouter-skills and push this branch to its main as a fast forward (if main moved, merge it in with a merge commit, rerun node scripts/check-catalog.mjs, then push; never force). 2. For each id in ${JSON.stringify(A.ids)}: node scripts/release.mjs <id> (dry run, read it), then node scripts/release.mjs <id> --execute --work ${A.work} --biorouter ${A.biorouterBin}. 3. node scripts/verify-release.mjs <id> for each id. 4. Commit the release.sha256 changes release.mjs wrote into catalog.json (plain message, no trailer) and push. Report each id's version, release URL and SHA-256, and the pushed commit.`, { label: 'release', phase: 'Release', schema: RESULT })
if (!rel || rel.status !== 'done') return { stopped: 'release did not finish', rel }

phase('Published review')
const reviews = (await parallel([1, 2].map(n => () => agent(`${COMMON}
Independent review ${n} of the PUBLISHED skill zips. Released: ${JSON.stringify(rel.items)}
For each id: download https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-<id>/<id>.zip into a fresh empty folder (never inside a repository), run node scripts/verify-release.mjs <id> from ${A.skillsWt}, and read the archive: SKILL.md frontmatter (name, version, licence), LICENSE-UPSTREAM and its copyright lines for adapted material, no secrets, tokens, personal data or real patient data, no file that runs on install, and content equal to the folder at the pushed commit. Edit nothing. pass=true only with no high and no medium finding. Return the schema.`, { label: `published review ${n}`, phase: 'Published review', schema: VERDICT, model: 'sonnet' })))).filter(Boolean)
if (reviews.length < 2 || reviews.some(v => !v.pass)) return { stopped: 'a published zip review did not pass: hold the BAAM PR and bring the findings to the owner', rel, reviews }

phase('BAAM PR')
const pr = await agent(`${COMMON}
Open the BAAM pull request for these skill releases: ${JSON.stringify(rel.items)} (skills commit ${(rel.commits || []).slice(-1)[0] || 'the pushed main'}).
Setup: git -C ${A.biorouterRepo} fetch origin; git -C ${A.biorouterRepo} worktree add ${A.baamWt} -b ${A.baamBranch} origin/main. Work only there, and source bin/activate-hermit first. Follow docs/extensions/baam-catalog.md ("Common changes").
1. node landing/scripts/sync-skill-catalog.mjs --skills-repo ${A.skillsWt} --commit <the pushed skills commit>.
2. landing/baam.html: each changed card's data-version, licence and description; a card for each new skill; for a merge, the old id in the target card's data-tags and a line in the Moved and retired list; every hand written count.
3. ${A.jobsChanges ? `landing/catalog/jobs.json: ${A.jobsChanges}` : 'landing/catalog/jobs.json: owner changes for any required job a released item now owns.'}
4. node landing/scripts/build-registry.mjs (it writes landing/registry.json, ui/desktop/src/components/baam/registry.fallback.json and crates/biorouter/src/privacy/registry_private.rs; never hand edit them or add a registry field).
5. Checks, each with its exit code: just check-registry; just check-privacy-registry; node landing/scripts/check-catalog.mjs --skills-repo ${A.skillsWt}; cd ui/desktop && (test -d node_modules || npm ci) && npx vitest run src/components/baam/registry.test.ts src/components/baam/search.test.ts; bash scripts/check-no-goose-block.sh.
6. Commit (plain message, no trailer), push the branch, open the PR with gh pr create (body: what changed per skill, versions and digests, the checks run, and any change to EXPECTED_PRIVATE_EXTENSIONS under "Needs human review"; end the body with the line 🤖 Generated with [Claude Code](https://claude.com/claude-code)). Do not merge. Return the schema with pr_url.`, { label: 'BAAM PR', phase: 'BAAM PR', schema: RESULT })
if (!pr || !pr.pr_url) return { stopped: 'no BAAM PR', rel, reviews, pr }

phase('Verify')
let v = await agent(`${COMMON}
You are a careful verifier of the BAAM pull request ${pr.pr_url} (worktree ${A.baamWt}). Report: ${JSON.stringify(pr)}
Check against primary sources, not the report: each released zip's SHA-256 against the catalog mirror's release.sha256; card data-version, licence and description against the released SKILL.md; the mirror equals catalog.json at the named skills commit (node landing/scripts/check-catalog.mjs --skills-repo ${A.skillsWt}); node landing/scripts/build-registry.mjs --check; counts on the page. Edit nothing. pass=true only with no high and no medium finding. Return the schema.`, { label: 'verify PR', phase: 'Verify', schema: VERDICT, model: 'sonnet' })
if (!v || v.unsure) v = await agent(`${COMMON}
Verify the BAAM pull request ${pr.pr_url} (worktree ${A.baamWt}) against primary sources: zip digests, card text against released SKILL.md files, the mirror against the skills commit, build-registry --check, page counts. Edit nothing. Return the schema.`, { label: 'verify PR opus', phase: 'Verify', schema: VERDICT })

phase('Address')
const fin = await agent(`${COMMON}
You opened the BAAM pull request ${pr.pr_url} on ${A.baamBranch} (worktree ${A.baamWt}). Verifier verdict: ${JSON.stringify(v)}
Fix every high and medium finding and the cheap lows, rerun the checks, commit (no trailer) and push. Then poll gh pr checks every few minutes until the required checks finish: test (ubuntu-latest), test (macos-latest), test (windows-latest), Unit tests (vitest), no-ai-coauthor. Fix failures your change caused. Do NOT merge: the orchestrator merges after reading this report. Return the schema with each check's state in summary.`, { label: 'address and CI', phase: 'Address', schema: RESULT })
return { rel, reviews, pr, verdict: v, finish: fin }
