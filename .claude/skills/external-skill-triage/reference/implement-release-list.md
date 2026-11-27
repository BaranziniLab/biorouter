# Implementation, release and the BAAM listing

> **What this is.** The steps that turn a final triage plan into released skills and extensions and a merged BAAM pull request: the worktree, per item ownership, staged catalog rows, the per item review, the integrator, the release, the published artifact review and the site change.
> **Status:** Current, as run for the 2026-11 BAAM refresh and the phi-deidentification release.
> **Audience:** the orchestrator and the implementer, reviewer, integrator, releaser and BAAM agents it dispatches.

## Before you start

- The final plan exists (`plan/triage-plan.json`), every owner decision it depends on is recorded, and items waiting on an open decision are held.
- No other campaign is halfway through a release on biorouter-skills main. Fetch and read its log first; wait for an in-flight release and its digest commit to land.
- The usage budget has room. An implementation wave of dozens of items is a heavy workflow.

## The skills worktree

Never commit in `~/Desktop/biorouter-skills`. Make a worktree (or a fresh clone) from the remote main:

```bash
git -C ~/Desktop/biorouter-skills remote -v          # pick the remote that points at BaranziniLab/biorouter-skills
git -C ~/Desktop/biorouter-skills fetch <remote>
git -C ~/Desktop/biorouter-skills worktree add ~/biorouter-runs/<campaign>/skills-wt \
  -b triage/<topic>-<yyyy-mm> <remote>/main
```

The campaign's staging folder (`~/biorouter-runs/<campaign>/staging/`) stays outside the worktree.

## One implementer per item

- **Ownership.** Each item lists the folders it may touch. An implementer edits only those, never `catalog.json`, `README.md` or `NOTICE`, and never another item's folder. Items that share a folder run one after another (the implement script groups them).
- **The catalog row.** The implementer writes its row to `staging/<wid>/<id>.json`: a complete row for a new id, or only the fields it changes for an existing id. `node scripts/merge-rows.mjs` merges them later, uniting aliases and boundaries and keeping history.
- **Adapted material.** Copy at the pinned commit, add `LICENSE-UPSTREAM`, fill the row's `source` block and `changes`, and set the SPDX `licence` (see [licence-policy.md](licence-policy.md)).
- **Ideas only material.** Write from primary sources in original wording; name the source in the row so the reviewer can run the similarity check.
- **Security conditions.** Every required change from the security review is an acceptance criterion. Code gets a test per condition; data tools run on invented data under `sandbox-exec` with the network denied.
- **Version and history.** Patch for a fix, minor for new content or a replacement; the same version in the row and in every `SKILL.md` of the item.
- **Self checks.** `node scripts/build-zip.mjs <id>` builds, and `node scripts/merge-rows.mjs --dry-run <staging file>` accepts the row. The full catalog check waits for the integrator, because other items are still in flight.
- **No commits.** Implementers leave their work uncommitted in their own folders.

## One independent review per item

A Sonnet reviewer that did not write the item checks, with lenses:

1. Quality and method correctness: would a researcher get the right answer by following it?
2. Licence: no copied text from an ideas only source (similarity check against each named source), attribution complete for adapted sources.
3. Security: every required change met, no new outbound action without a yes, nothing in [security-checklist.md](security-checklist.md) reintroduced.
4. Catalog: row shape, job, boundaries, aliases, versions, and the dry run merge.

Pass means no high and no medium finding. On a fail, an Opus fixer addresses the findings and a fresh reviewer re-reviews with the earlier findings, told to look for what the fix opened. Cap the rounds (three worked) and send the residual list to the owner at the cap.

## One integrator

One Opus agent, after every item's review has ended:

1. `node scripts/merge-rows.mjs --dry-run <staging>/*/`, then without `--dry-run`.
2. `node scripts/write-stubs.mjs` for merged and retired items, then `node scripts/render-readme.mjs`.
3. The checks, each with its exit code reported:
   - `node scripts/check-catalog.mjs`
   - `node scripts/check-catalog.mjs --jobs <biorouter worktree>/landing/catalog/jobs.json --extensions <biorouter worktree>/landing/catalog/extensions.json`
   - `node scripts/render-readme.mjs --check` and `node scripts/write-stubs.mjs --check`
   - `node --test scripts/`
4. Commit per item (or per group of items that share folders), plain messages, no trailer. Do not push: pushing is the releaser's first step.

`baseline_violations` may record a failure a later item removes; `node scripts/baseline.mjs` maintains it. Never use it to hide a real failure.

## Release: a separate agent, behind an owner flag

Keep release behind a flag in the script (`releaseApproved`, default false) that the owner flips. The releaser never fixes a security finding.

1. Push the worktree branch to main of BaranziniLab/biorouter-skills (fast forward or a merge commit; never force).
2. For each id: `node scripts/release.mjs <id>` (dry run), then `node scripts/release.mjs <id> --execute --work <tmp dir> --biorouter <released biorouter CLI>`. The work folder keeps `counts-before.tsv`, the download counts the usage dashboard needs, and the rollback archive. `--biorouter` installs the new archive in a throwaway `BIOROUTER_PATH_ROOT`, fresh and over the previous one.
3. `node scripts/verify-release.mjs <id>` for each id.
4. Commit the `release.sha256` changes that release wrote into `catalog.json`, and push.
5. Never delete a release or a tag. Asset names stay `skill-<id>` and `<id>.zip`: the usage dashboard's classifier reads them (CLAUDE.md, "Usage dashboard").

## Review of the published bytes

Two Sonnet reviewers per release, independent of the releaser: download the published zip into a fresh folder, run `verify-release.mjs`, confirm `LICENSE-UPSTREAM` and the frontmatter licence, confirm the content is what the per item review passed, and look for secrets and personal data. Nothing is advertised until both pass.

## The BAAM pull request

In a worktree of the biorouter repo from `origin/main`, following [`docs/extensions/baam-catalog.md`](../../../../docs/extensions/baam-catalog.md) ("Common changes"):

1. `node landing/scripts/sync-skill-catalog.mjs --skills-repo <skills checkout> --commit <pushed skills sha>`. The commit must already be on the remote, or the deploy cannot check the mirror.
2. In `landing/baam.html`: each changed card's `data-version`, licence and description; new cards; the old id in the target card's `data-tags` for a merge; the Moved and retired list; every hand written count. For extensions, also `landing/baam-details.js`.
3. `landing/catalog/jobs.json` owner changes for any required job an item now owns, in the same change.
4. `node landing/scripts/build-registry.mjs`. It writes three files together: `landing/registry.json`, `ui/desktop/src/components/baam/registry.fallback.json` and `crates/biorouter/src/privacy/registry_private.rs`. Never hand edit them, and never add a field to `registry.json` (registry v2 is parsed with `deny_unknown_fields`).
5. The checks: `just check-registry` and `just check-privacy-registry` (after `source bin/activate-hermit`), `node landing/scripts/check-catalog.mjs --skills-repo <skills checkout>`, the desktop registry tests (`ui/desktop/src/components/baam/registry.test.ts` and `search.test.ts`, after `npm ci` in `ui/desktop` if it has no `node_modules`; never `npm install`), and `bash scripts/check-no-goose-block.sh`.
6. Commit without a trailer, push, open the PR (no @mentions; the body may end with the Claude Code line). A change to `EXPECTED_PRIVATE_EXTENSIONS` in `build-registry.mjs` goes under "Needs human review" in the body.
7. A Sonnet verifier checks the PR against primary sources: release assets and digests, card text against the released content, counts.
8. Wait for the required checks: `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)`, `Unit tests (vitest)` and `no-ai-coauthor`. Merge with `gh pr merge <n> --merge --match-head-commit <sha>` once they pass, unless the owner asked to approve the merge.
9. If the Pages deploy fails with "Fetching artifact metadata failed", `gh run rerun <id> --failed` clears it. Confirm https://biorouter.ucsf.edu/registry.json lists the new versions.

Site text outside the catalog (for example a news line on the landing page) goes straight to main by the owner's rule; registry and catalog changes always go by pull request.

## New extensions

Each new extension is its own project in its own BaranziniLab repository:

- Build it from the BRXT adapter kit (`scripts/brxt-adapter-kit/`, documented in [`docs/extensions/brxt-adapter-kit.md`](../../../../docs/extensions/brxt-adapter-kit.md)): `bin/new_adapter.py` renders the repository from one spec, `bin/vendor_kit.py` vendors the kit, and the template's release workflow publishes `<id>.brxt`, `<id>-X.Y.Z.brxt` and `SHA256SUMS` for a tag equal to `v` plus the manifest version.
- Wrap upstream MCP servers or libraries at pinned versions, and record third party components, their licences and data terms in the catalog row.
- Run the leak suite and an isolated install (`BIOROUTER_PATH_ROOT` in a throwaway folder, `BIOROUTER_DISABLE_KEYRING=true`), fresh and over any older version, and confirm the tools listed.
- Give it an independent security review before release. `safe_http.py` is security code that needs a person's review after every change.
- Creating the public repository and pushing the release tag are outward actions: list them for the orchestrator and the owner to approve.
- Then its card, `extensions.json` row, `baam-details.js` entry and a new private set entry if it is private, all in the BAAM pull request.

## Related documentation

- [../SKILL.md](../SKILL.md): stages 8 and 9 and the rules they follow.
- [verdicts-jobs-and-catalog.md](verdicts-jobs-and-catalog.md): catalog rules and versions.
- `README.md` of BaranziniLab/biorouter-skills, section "The catalog".
