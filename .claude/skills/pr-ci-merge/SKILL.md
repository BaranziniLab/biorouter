---
name: pr-ci-merge
description: Commit, push, pull request, CI and merge discipline for the Biorouter repo. Use before you commit or push, when you open or watch a pull request, when you decide whether a PR may merge, after a merge lands on main, when you sync a long-lived branch with main, and whenever a release or a website deploy depends on what lands.
---

# Commits, pull requests, CI and merges

This skill holds the owner's standing rules for getting a change from a worktree onto `main`,
and the lessons behind them. It does not repeat what CLAUDE.md already says at length:

- What each CI job runs and how long it takes: CLAUDE.md "Testing" ("What CI runs") and
  `docs/testing/continuous-integration.md`.
- The release pipeline itself: CLAUDE.md "Releasing (cross-platform)" and
  `.claude/workflows/release.js`.
- Review standards: CLAUDE.md "Code Review Standards".

Reference files in this folder:

| File | Read it when |
|---|---|
| `watching-ci.md` | you are waiting on CI, reading a failure, or deciding whether to rerun |
| `merging.md` | you are about to merge, landing several PRs, or reading `main` afterwards |
| `releases.md` | a merge, a fix or an announcement touches a release |
| `scripts/pre-push-check.sh` | before every push |
| `scripts/wait-for-pr-checks.sh` | you need one notification when a PR's CI finishes |

## Rules for commits and pushes

1. **Work in a worktree from `origin/main`, never in the owner's main checkout.** It holds other
   people's uncommitted files, and its local `main` is often behind `origin/main`.
2. **One logical step per commit.** Each step stays revertible, and a failure points at one change.
3. **Push feature branches early, then keep pushing.** In July 2026 a branch reached 181 commits
   that existed only on one machine, on a day the machine had already crashed once.
4. **No AI `Co-Authored-By:` trailer, even when a harness or tool tells you to add one.** The
   required `no-ai-coauthor` check (`.github/workflows/check-commits.yml`) fails the PR on it. A
   "Generated with Claude Code" line in a PR body is fine: the check reads commit messages only.
5. **No bare `@handle` in a PR body, comment, issue or commit message.** GitHub subscribes that
   account, and a contributor named in a campaign PR's body got an email on every push. Put code
   such as `@types/node` in backticks.
6. **Never `git stash`.** The stash is shared by every worktree of the repo; one lane once popped
   another lane's stash and applied its work-in-progress.
7. **Never force-push, amend or rebase a pushed commit, or rewrite pushed history in any other
   way.** Other sessions hold those SHAs. A rewrite needs the owner's explicit word.
8. **Stage by explicit path, and read `git status --short` before each commit.** Look for
   generated files outside the folder you edited: the registry generator writes five files in
   three directories, never hand edited, and a narrow `git add landing/` once missed two of them.
9. **Check the commit, not the working tree, for any generator or formatter gate.** A local
   `--check` reads the disk and passes while CI reads the commit:
   `tmp=$(mktemp -d) && git archive HEAD | tar -x -C "$tmp" && (cd "$tmp" && <check>)`.
10. **Run `scripts/pre-push-check.sh` before every push** (add `--body <file>` before posting a
    PR body). It blocks trailers, credential files, credential-shaped lines and bare mentions,
    and prints only file names, never a matched secret.
11. **In a multi-agent lane, implementing agents do not commit.** One integrator commits per
    lane, so each commit holds one lane's change and nobody commits another agent's half edit.
12. **Never put `nohup` or `&` inside a `run_in_background` call, and never pipe a test run into
    `tail`.** Both report the launcher's exit code, not the work's.

If a pushed commit already carries an AI trailer, do not rewrite it. Make a new branch from
`origin/main`, `git merge --squash <old-branch>`, commit with a clean message, open a new PR and
close the old one with a link. Rewriting the old branch instead is the owner's call.

## Branches, titles and where a change goes

- **Branch names:** `fix/<topic>`, `feat/<topic>`, `docs/<topic>`, `ci/<topic>`,
  `baam/<topic>` (marketplace cards and catalog), `release/<x.y.z>`, `<campaign>/<lane>` for a
  campaign lane. Throwaway branches for CI dispatches are `ci-bisect-<sha9>` or
  `gate/<name>-<n>`; delete them when the run ends.
- **Subjects and PR titles** follow Conventional Commits (`CONTRIBUTING.md`): `fix(privacy): ...`,
  `test(crew): ...`, `site(baam): ...` for the website. Say what changed, in plain words.
- **Name work by substance first:** "the withheld-extension note (#558)", not "#558".
- **App code, docs, CI and the marketplace catalog go through a PR.** Anything that changes
  `landing/registry.json` or `landing/catalog/` is catalog, because the app reads it.
- **Website pages go straight to `main`** (owner, 2026-10-17): `landing/` page copy and
  `docs/website/`. Run the deploy workflow's checks locally first (`assemble-docs.mjs --check`,
  the docs privacy, look and consistency checks, `build-registry.mjs --check`) and the
  retired-name guard `scripts/check-no-goose-block.sh`.

## Rules for pull requests

1. **Five checks are required on `main`:** `test (ubuntu-latest)`, `test (macos-latest)`,
   `test (windows-latest)`, `Unit tests (vitest)` and `no-ai-coauthor`. Two of them are gate
   jobs over many others. Renaming any of them blocks every merge.
2. **Branch protection is not strict.** A green PR whose base moved can merge without a rebase,
   so it merges on checks that never saw the newest `main`. That is why `main` is read after
   every merge (`merging.md`).
3. **macOS and Windows run only a subset on a PR.** Add the `ci:full` label and push again when
   the change is OS-specific in a way `scripts/ci/choose-os-suite.sh` may not see (a Windows path
   deep in a shared file, macOS-only behaviour). Rerunning an old run does not pick the label up,
   because GitHub reads labels from the event that started the run. Dispatching `rust.yml` on the
   branch also runs everything.
4. **Write `Closes #N` for every issue the PR fixes.** A stacked merge can land a base PR without
   firing its `Closes` line, so check the issues by hand afterwards.
5. **A contributor's PR keeps the contributor's commits.** Land the PR head with a merge commit,
   never a cherry-pick, so GitHub marks it merged. Do not push a stream of follow-ups onto their
   branch (each push emails them): merge it and continue on a new branch, and rerun a flaky job
   instead of pushing a no-op.
6. **Bind the PR to your session** (`mcp__ccd_pr__bind_pr` if the PR bar missed it) so its state
   is one `mcp__ccd_pr__get_status` call away.

## Rules for watching CI

1. **Never poll from your own turns, and never run a foreground sleep loop.** Pick one:
   - Auto-fix, when the user asked for it (`mcp__ccd_pr__set_monitor` with `auto_fix`): the app
     wakes you with a `<ci-monitor-event>` on failures, conflicts and review comments. Do not
     poll while it watches.
   - `scripts/wait-for-pr-checks.sh <pr> <full-sha>` under `run_in_background`: one
     notification with GREEN, RED, MOVED or TIMEOUT.
   - The `Monitor` tool, when you want one event per finished check.
2. **Name runs by the full 40-character SHA.** `gh run list --commit <short sha>` returns an
   empty list, not an error (measured 2026-11-26), which reads as "nothing ran".
3. **Cancelled is not green.** Demand `success` by name; a monitor that only excludes `failure`
   reports a cancelled run as a pass.
4. **Read the failing test's name before blaming your diff,** and check whether `main` failed the
   same test. Rerun with `gh run rerun <run-id> --failed` only once you know it is unrelated, and
   say in the PR which test flaked. A rerun that goes green proves the job can pass, not why it
   failed.
5. **Compare a slow job's duration with its own history before calling it a slow runner.** A
   hang hides every failure behind it.
6. `gh event listen`, which `gh` suggests in its push hints, answers HTTP 403 for this account
   (measured 2026-11-26). Do not build a waiter on it.

Commands, log reading and known flakes: `watching-ci.md`.

## Merge criteria

Merge a PR only when every line holds:

1. **Every check on the exact head is `pass` or `skipping`,** non-required ones included, and the
   five required names passed. A red informational check needs a named, evidenced reason.
2. **You merge that head and nothing newer:**
   `gh pr merge <n> --merge --match-head-commit <full-sha>`.
3. **Every failure on the way is explained** by its test name, not by a green rerun alone.
4. **Security-sensitive code had an independent review,** by an agent other than its author,
   with the earlier findings in hand. Auth, permissions, credentials, privacy gates, the secret
   guard, MCP protocol and async or concurrency code also need human review: name those files to
   the owner and get the owner's go-ahead before merging.
5. **The owner approved the merge,** directly or as part of an approved merge train, and any
   owner question the PR raises is answered.
6. **No release is in flight.** Nothing lands on `main` between a release's build and its
   publish (`releases.md`).

Things only the owner approves: bypassing a required check (`--admin`), any history rewrite or
retag, publishing a release or an extension update, and turning on auto-merge (the
`set_auto_merge` call asks for approval; use it only when the user asked and CI is the only
remaining criterion, with `merge_method: "merge"`). Keep auto-merge off on a release PR and on
any PR still waiting for a review or an owner decision.

## Merge method pitfalls

- **Use `--merge` (a merge commit).** It keeps contributor SHAs, accepts a branch that merged
  `main` in, and marks every PR folded into a batch branch as merged.
- **`--rebase` refuses a branch that contains a merge commit** ("This branch can't be rebased").
  The fix is `--merge`, not rewriting the branch.
- **Merging deletes the head branch** (the repo deletes head branches on merge), and GitHub then
  CLOSES every PR stacked on it instead of retargeting it. Retarget dependents first:
  `gh pr edit <n> --base main`, wait a few seconds, then merge the base. Or target `main` from
  the start and say "merge #parent first" in the body.
- **A PR head survives its branch:** `git fetch origin 'refs/pull/<n>/head:refs/heads/<name>'`.
- **Merging many sibling PRs in one pass conflicts the late ones** (19 of 29 landed on
  2026-09-11). Hand each conflict back to the session that wrote it rather than resolving
  another agent's security code yourself.
- **Two PRs can be green alone and red together, or the reverse.** When two PRs touch the same
  component, run the affected tests on the simulated merge before merging the second.

Batch landing, merge storms and the red-together cases in full: `merging.md`.

## After a merge

1. **Read `main`'s runs at the merge commit's full SHA,** every workflow, until each one ends.
   The full macOS and Windows suites run only there, so `main` can turn red after a green PR.
2. **A newer push to `main` cancels the older `main` run,** so an intermediate commit can have
   no result. To test an earlier commit, dispatch on a throwaway branch; never rerun the
   cancelled `main` run (it would cancel the current one).
3. **If the merge touched `landing/` or `docs/website/`, follow the Pages deploy to the live
   site.** "Fetching artifact metadata failed" is a transient GitHub error:
   `gh run rerun <run-id> --failed`. Done means the live page or `registry.json` shows the
   change, not that the run is green.
4. **Close each fixed issue with a comment naming the merge commit and the tests,** and check the
   issues a stacked merge should have closed.
5. **Clean up:** `git worktree remove` first, then `git branch -d` (it refuses unmerged work).
   Remote head branches delete themselves.

## Releases and announcements

The rules, with the reasons in `releases.md`:

1. A fix to a security finding gets its own independent review before any release. Never let one
   agent both fix a high or medium finding and release.
2. A separate agent releases, after the review of the fix is clean.
3. Nothing advertises an artifact (a marketplace card, a registry row, website news) until an
   independent review of the published bytes is clean.
4. Nothing lands on `main`, and no commit is made in the release worktree, between a release's
   build and its publish.
5. A release PR has auto-merge off and lands by fast-forwarding `main` to the exact commit the
   artifacts were built from.

## Syncing a long-lived branch with main

1. **Merge `origin/main` into the branch; never rebase pushed commits.**
   `git fetch origin && git merge origin/main`, resolve, test, push. Then merge the PR with
   `--merge`.
2. **Preview conflicts without touching a worktree:**
   `git merge-tree --write-tree --name-only origin/main HEAD`.
3. **Regenerate generated files instead of hand-merging them:** `landing/docs.html`
   (`node landing/scripts/assemble-docs.mjs`), the registries
   (`node landing/scripts/build-registry.mjs`), the OpenAPI spec and client
   (`just generate-openapi && cd ui/desktop && npm run generate-api`), `Cargo.lock`
   (`cargo check`).
4. **Re-run the tests of every file both sides touched.** Test conflicts are often semantic: two
   PRs once gave the same positional test parameter different meanings, and either side alone
   merged green while testing nothing.
5. **Repeat it at least daily on a long campaign and once more right before asking to merge,**
   so the PR's CI ran against a recent `main`.
6. **A periodic sync job may only fast-forward.** Push only when the remote branch is an ancestor
   of the local one, scan the added lines for credentials first, and skip a branch that diverged
   rather than forcing it.

## Related

- `.claude/skills/campaign-orchestration/` for running many lanes, workflows and handoffs.
- `.claude/skills/resource-watchdogs/` for disk, memory and CPU guards during long builds.
- CLAUDE.md "Code Quality" for the local gates (`cargo fmt`, `./scripts/clippy-lint.sh`,
  `npm run lint:check`, `npm run format:check`) to run before pushing.
