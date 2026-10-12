# Codex simplicity redesign: handoff

> **What this is.** Everything the next agent needs to finish the October 2026 redesign: where the
> work stands, what is left in order, how to run it, and the traps the run hit.
> **Status:** Current. Rewritten 2026-10-11 by the second coordinator: implemented, vision rounds 1
> to 4 done, rounds 5 and 6 running, final gate green at `0cb5ab465`, no pull request open yet.
> **Audience:** the agent or developer who takes over the redesign.

The redesign is defined by three documents beside this one: the owner's requirements
([requirements-and-plan.md](requirements-and-plan.md), R-01 to R-34), the implementation spec
([implementation-spec.md](implementation-spec.md), sections 0 to 11) and the tracker
([progress.md](progress.md)). Read sections 9 to 11 of the spec before anything else: they hold
the owner's latest rules (Tools switcher at the bottom, Effort slider, fills over lines, the
wordmark as the logo, opening is not activity, the retired names, the composer card on the page
ground, the big Knowledge drop zone).

## Where everything is

| Thing | Location |
|-------|----------|
| Branch | `claude/crew-landing-redesign-91b46e`, base `30e1f0622`, 728 commits ahead of `main` at `0cb5ab465` and pushed through it. No pull request yet. |
| Worktree | `/Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e` |
| Run directory (outside the repo) | `~/biorouter-runs/redesign-2026-10-09/` |
| Owner's verbatim messages 1 to 18 | `REQUIREMENTS.md` in the run directory. The single most important input. |
| Coordinator rulings 1 to 21 | `requests/coordinator/RULINGS-GIVEN.md`; answered questions in `RULINGS-NEEDED-answered-*.md`. No question to the owner is open. |
| Research and audits | `research/*.md`, `audit/*.md`, `icons/` (icon proposals and contact sheets) |
| Old app screenshots | `baseline/*.png` and `audit/baseline-screens.md` |
| Per workstream notes (commits, decisions, deviations) | `impl/WS-*.md` and `impl/<WS>-tools/`; the Windows folder move in `impl/WS-BRAND.md` and `impl/WS-BRAND-tools/winmove/map.md` |
| Gate logs | `impl/gate-*.log` (summary) and `impl/gate-*.full.log` (per step); the last green one is `impl/gate-final-2.log` |
| Vision reports | `vision/round-<n>/V1.md` to `V5.md` (round 4 has no `V5.md`: the site was not reviewed) with their screenshots and probes; the instances in `vision/instances.json` |
| Requests between workstreams | `requests/<OWNER>/*.md` (pending ones have no `.done`, `.rejected` or `.withdrawn` suffix) |
| Pull request description (draft) | `PR-BODY.md` in the run directory |
| Workflow scripts | `workflows/*.js` |
| Dev app launcher | `~/biorouter-runs/launch-dev-gui.sh` (see the memory note `dev-gui-launch-script.md`) |

## State at handoff

### Done and committed

- Every requirement is implemented except the vision loop itself (R-23), which is still running.
  The tracker marks 21 of the 33 Done, verified in the running app with no open high or medium
  finding after vision round 4, and R-31 Done on macOS; eight wait for vision round 5, and R-32
  and R-33 for a vision check.
- R-25 to R-28 (owner messages 13 and 14) landed, and the R-27 fill sweep reached every workstream.
  The sidebar listing reads a whole reordered list in one request (`MAX_SIDEBAR_SESSION_LIMIT` is
  2,000), and History's listing query dropped its join and grouping (`0403b8d63`).
- R-29 to R-31 (owner messages 15 and 16) landed:
  - The wordmark is the only logo, and the unreferenced logo images are deleted.
  - Opening a chat no longer moves it; a rename and a turn still do (ruling 16 sets the scope).
  - The retired names are gone from the tracked tree except NOTICE, and a repo guard keeps them
    out.
  - On Windows the app folder moved to `%APPDATA%\UCSF\biorouter`.
- R-32 and R-33 (owner message 17, 2026-10-11) landed: the composer card is the page ground with
  its hairline again (`d3be11a4b`, `a5fa59b6f`, the site's mockups in `a5cd7046c`, and the message
  edit card by ruling 21 in `95d838a25`), and the Knowledge drop zone fills the Sources rail again
  (`5055fe64d`).
- Vision rounds 2 to 4 ran on four sandboxed app instances (plus the website in rounds 2 and 3):
  40, 28 and 19 findings, of which 12, 6 and 6 were high or medium. Round 4 found no high. Its six
  medium findings all have fixes committed:
  - V1-R2-2: the sidebar draws only the rows near the viewport.
  - V2-R4-1: the context ring.
  - V3-R4-1: New schedule's name error.
  - V4-R4-1: the Lead/worker focus ring.
  - V4-R4-2: Report a problem on ModalShell.
  - V4-R4-3: the shared Select in onboarding.
- Final gate round 2 at `0cb5ab465` is green: `lint:check` (1,148 contrast assertions),
  `format:check`, 946 test files (13,095 tests), the site and Crew checks, 175 site tests,
  `cargo fmt --check`, `./scripts/clippy-lint.sh`, 509 session tests, `privacy_guard_wiring`,
  every `biorouter-server` target (with `BIOROUTER_DISABLE_KEYRING=true`), and a rebuilt
  `target/debug/biorouterd`.

### Not done yet, in the order to do it

1. **Vision rounds 5 and 6** are running. Read their reports in `vision/round-5/` and
   `vision/round-6/`. They must confirm the six round 4 medium fixes above; R-01, R-06, R-07,
   R-13, R-15, R-17, R-18 and R-28 stay "Implemented; vision round 5 pending" until they do, and
   R-32 and R-33 stay "vision check pending" until a round sees them on a build that has them.
   Route what they find to its owner, and repeat until each group is clean twice.
2. **The first run redesign (R-34, owner message 18)** has no commit yet. Owner: WS-SETTINGS-M
   (`ProviderGuard.tsx`, `components/onboarding/**`). The target and its limits are in spec section 11:
   the same standard as the rest of the app, privacy copy and logic unchanged (class only changes,
   ruling 15), and a review in a sandbox with no provider
   (`~/biorouter-runs/launch-dev-gui.sh start <worktree> <run> <cdp-port> --no-provider`) that
   never downloads a local model and never enters a key. Then it joins the vision rounds.
3. **Round 4 low findings** still open (no commit after round 4 names them):
   - V1: History takes 0.95 s to show its rows (V1-R3-2), the auto collapse at the 1120 rung
     slid in 3 of 13 resize crossings (V1-R4-1), and the list's top edge cuts text when scrolled
     (V1-R4-2).
   - V2: the summary popover runs to the window's bottom edge (V2-R4-2), deleting a base reads
     "Removed KB base" (V2-R4-3), and the active tab can sit under the strip's end fade (V2-R4-4).
   - V3: V3-R4-2 to V3-R4-5. V3-R4-2 is the Knowledge tier badge's rest fill, a privacy UI
     presentation change for the review list.
   - V4: V4-R4-4 to V4-R4-6.
4. **Final gate again** after the last fix, the same steps as `impl/gate-final-2.log`, ending
   with a clean `git status`.
5. **`CLAUDE.md`** (ruling 3): at `0cb5ab465` it carried only the PreviewBar and rung 0 bullets.
   The coordinator docs pass `cc710982a` (2026-10-11) wrote the rest (the 28px sidebar rows and
   the view menu, the composer and Tools popover, the icon registry and lock badge, the fill rule
   and the R-32 composer card, the Slider primitive, the R-30 activity rule, the repo guard), and
   a claim pass checked it against the code. Re-measure any value a round 5 or 6 fix moves.
6. **[progress.md](progress.md):** mark the pending rows Done as rounds 5 and 6 pass, and log
   each round.
7. **A Windows run of the folder move** before release, with the checklist in `PR-BODY.md`
   (from `impl/WS-BRAND.md`). Nothing in CI exercises the real Windows layout.
8. **Pull request** against `main`, using `PR-BODY.md`. Do not merge: the landing site deploys
   when `main` changes, and the owner approves publication.

## Needs human review in the pull request

The full list, with reasons, is in `PR-BODY.md`. In short:

- The session listing SQL changed shape twice: the sidebar's keyset listing (`aceb5d6e5`) and the
  full list behind History and `biorouter session list` (`0403b8d63`). `MAX_SIDEBAR_SESSION_LIMIT`
  went from 50 to 2,000. The privacy predicates are unchanged.
- Privacy UI presentation moved: the Knowledge tier badge sits in the band, the model chip's
  privacy lines and the DR-17 disclosure moved into the model menu, and two class only restyles
  (ruling 15) turned outlined boxes into fills.
- Text fields lost their resting border (an accessibility trade-off recorded in `design.md` Part 9;
  forced colours keep borders).
- R-30 runs the cold restore bind (Gate A) inside an opening scope. Only the `updated_at` stamp
  changes; the `WHERE` clause does not.
- The Windows app folder move is a data migration that has not run on Windows.
- The private-environment filters lost the retired prefix (owner approved; security adjacent).
- The LICENSE appendix line was removed (owner decision, ruling 19); NOTICE is kept.
- The sidebar draws only the rows near the viewport.

## How to resume cheaply

- Run at most six to eight agents at once. Fourteen at once hit the account's usage limit within
  35 minutes three times; each hit kills every agent in flight.
- Every agent must commit small steps and append notes to `impl/<WS>.md`; a resume prompt tells
  the next agent to read its notes, its commits and its uncommitted edits.
- Give agents spec line ranges instead of the whole 3,371 line spec.
- Sonnet is enough for inbox processing, gates and most fixes; keep Opus for the vision reviewers
  and for the summary rail, sidebar, primitives, composer and motion work.

## Traps the run hit

- **`git commit -a` in a shared worktree** sweeps in other agents' unfinished files. Always add
  explicit paths. A plain `git commit` also takes whatever another agent has staged (it happened
  twice with staged deletions).
- **Commit with a trailing pathspec.** Other agents stage files in this worktree at any moment, so
  `git add <paths> && git commit -m "…" -- <paths>` is the only form that commits exactly your
  paths.
- **The worktree write block.** A hook refuses Edit and Write into this worktree from a session
  whose own worktree is a different one. Three delegate passes launched from another worktree could
  only queue patches. Launch every agent from a session whose worktree is this one (ruling 18).
- **`main.css` partial staging corrupted the file twice** (fixed in `09dcc0213` and `f3b7a8116`).
  Stage `main.css` whole; it may carry other workstreams' blocks, which is fine.
- **The gate's `biorouter-server` step needs `BIOROUTER_DISABLE_KEYRING=true`.**
  `tests/schedule_write_reach.rs` asserts that precondition and fails without it; `rust.yml` sets
  it. Bare `cargo test -p biorouter-server` is red on this machine for that reason alone.
- **A vite run from the repository root writes `public/pdfjs`** (the `pdfjsAssets.mts` plugin
  copies into the current directory). The root `.gitignore` covers it since `0cb5ab465`; before
  that it failed the gate's clean tree step.
- **Dev app instances:** start them one at a time, `chmod 700` the run's `electron` directory,
  dismiss the dependency and CLI update modals without installing anything, and know that
  `BIOROUTER_NO_HMR=1` means code changes need a restart (and a newly written Tailwind class only
  appears after one). The app runs `target/debug/biorouterd`, so rebuild it after any Rust change
  with `cargo build -p biorouter-server --bin biorouterd --features biorouter/privacy-test-auth`.
- **An occluded instance measures nothing.** A review window fully covered by the others reports
  `visibilityState` hidden and runs no animation frames; move it into view before timing motion.
- **Never close all agent-browser sessions**; close only your own by name.
- `vitest` can exit 1 with zero failed tests when a browser test's `browser.close()` teardown
  times out under load (`artifactCdnAssets`, and in the last gate `fillTokens.browser.test.ts`);
  read the summary line and rerun the file alone.

## Related documentation

- [Requirements and plan](requirements-and-plan.md): R-01 to R-34 and the owner decisions.
- [Implementation spec](implementation-spec.md): the rules, ownership and verification; sections 9
  to 11 hold the latest owner rulings.
- [Progress](progress.md): the live tracker and the dated log.
