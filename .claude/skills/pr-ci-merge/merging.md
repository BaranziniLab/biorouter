# Merging and reading main afterwards

Read this before you merge, when you land several PRs together, and after a merge. The merge
criteria themselves are in SKILL.md.

## The merge itself

```bash
n=<pr>; sha=$(gh pr view $n --json headRefOid --jq .headRefOid)
# Every check pass or skipping (prints nothing when that holds)
gh pr checks $n --json name,bucket --jq '.[] | select(.bucket!="pass" and .bucket!="skipping") | "\(.name)=\(.bucket)"'
gh pr merge $n --merge --match-head-commit "$sha"
gh pr view $n --json state,mergeCommit --jq '"\(.state) \(.mergeCommit.oid)"'
```

`--match-head-commit` makes GitHub refuse the merge if anyone pushed after you checked. Record
the merge commit's full SHA; it is what you read `main`'s CI by.

Repo settings measured 2026-11-26: merge commits, rebase and squash are all allowed, auto-merge
is allowed, head branches are deleted on merge, and `main` requires the five checks with strict
mode off and no linear-history rule.

### Why a merge commit

- It keeps every commit's SHA, so a contributor's PR shows as merged and their commits stay
  theirs.
- It accepts a branch that merged `origin/main` in, which is how long-lived branches are synced
  here (pushed history is never rebased).
- When one branch carries several PRs' heads, merging it marks every one of those PRs merged.

`--rebase` answers "This branch can't be rebased" on any branch containing a merge commit. Do
not rewrite the branch to satisfy it; merge with `--merge`.

## Stacked PRs

Merging deletes the head branch, and GitHub then CLOSES each PR whose base was that branch. It
does not retarget them from the CLI or the API, and a closed PR's base cannot be changed.

- **Before merging a base PR,** retarget each dependent: `gh pr edit <child> --base main`, then
  wait a few seconds for mergeability to recompute. The child stays mergeable because its branch
  already contains the base's commits.
- **Or target `main` from the start.** CI runs the same either way. Lead the body with "merge
  #parent first" and a `compare/<parent-branch>...<branch>` link to just this change.
- **Stacks can also merge bottom-up into their own bases,** deepest first; only a base merged
  into `main` closes its dependents.
- **If a dependent already closed:** push its last SHA back as the base branch
  (`git push origin "${sha}:refs/heads/<base-branch>"`), `gh pr reopen <child>`, retarget it,
  then delete the branch again. Reopening reruns the whole CI.
- A merged stack can land the base's change without firing the base's `Closes #N`. Check the
  issue states afterwards and close by hand, naming the commit.

## Landing many PRs

### Prefer one batch PR once heads are frozen

The November 2026 campaign landed five PRs faster as one batch than as a serial train:

1. Each owning lane freezes and pushes its final head.
2. An integration branch from `origin/main` merges each head as it is (merge commits, never
   cherry-picks), resolving conflicts once.
3. One CI cycle runs on the batch PR. It found real bugs the single-PR runs had missed: a huge
   allocation on a damaged audio file, a Windows path test, census collisions.
4. Merge the batch with a merge commit. GitHub marks every included PR merged.

### When merging a train one by one

- Merge in the agreed order, one at a time, each on green CI at its own head.
- Expect the later PRs to turn CONFLICTING as `main` moves under them. On 2026-09-11, one pass
  over 29 green PRs merged 19 and left 9 conflicting.
- Collect every conflict; do not stop at the first. Name the files without touching a worktree:
  `git fetch origin pull/<n>/head && git merge-tree --write-tree --name-only origin/main FETCH_HEAD`.
- Hand each conflicted PR back to the session that wrote it (find it by PR number in the session
  list) with the file list, which siblings already landed in those files and what they changed,
  the tests to rerun, and "push, do not merge". Do not resolve a conflict inside someone else's
  security gate yourself: half a gate can merge on stale-green checks.
- Expect a stack's base PR to go BLOCKED for a while after a child lands in it; its checks rerun.

### Green alone, red together (and the reverse)

- **Individually green, jointly red.** Two PRs each passed, and merged back to back they broke
  `main`'s vitest: one test queued a message that the next test then sent. A textual
  `git merge-tree` said "clean" and proved nothing about behaviour. When two PRs touch the same
  component, build the merge in a scratch tree and run the affected test files before merging
  the second.
- **Individually red, jointly green.** Two PRs each fixed half of one broken census assertion,
  so each stayed red until the other landed. Waiting for both to go green had no reachable state.
  The way out: build both heads on `main` in a scratch branch and run the failing test there,
  confirm it by a second method, get the owner's go-ahead, merge back to back with `--admin`,
  then check the real merged `main`. Tell the owner you bypassed a required check, why, and what
  evidence replaced it.

## Contributor PRs

- Land the contributor's head with a merge commit, so GitHub marks the PR merged. If a PR cannot
  be detected as merged, close it with a comment linking the commits that landed.
- Keep their code and their tests; follow-up changes go in your own commits.
- Every push to their PR emails everyone subscribed to it. When the campaign's work is unrelated
  to theirs, merge their PR and continue on a new branch; rerun a flaky job instead of pushing.
- Never put their handle in a PR body or comment.

## Reading main after a merge

1. **Find the runs at the merge commit's full SHA:**
   `gh run list --commit <full-sha> --json workflowName,status,conclusion`. Expect Rust,
   Frontend, Check Commit Messages, and Apps smoke or Deploy landing site when their paths
   changed.
2. **Wait for each to end** (a background waiter, `watching-ci.md`), then demand `success` by
   name. `main` runs the full macOS and Windows suites a PR did not, so a merge can turn `main`
   red. Whoever merged reads `main`'s run; in a campaign, the integration lane does.
3. **If `main` is red, name the failing test first.** `main` can be red from a known flake or an
   earlier merge. Check the previous `main` run before blaming the last merge.
4. **If the commit has no result of its own,** a newer push cancelled it (since 2026-11-14).
   Find the breaking merge by dispatching on throwaway branches at the earlier commits:

   ```bash
   sha=<earlier commit>
   git push origin "${sha}:refs/heads/ci-bisect-${sha:0:9}"
   gh workflow run rust.yml --repo BaranziniLab/biorouter --ref "ci-bisect-${sha:0:9}"
   ```

   A dispatch is keyed by its own run id, so it cancels nothing and nothing cancels it. Never
   rerun the cancelled `main` run: it rejoins `main`'s group and cancels the run in progress.
   Delete the branches afterwards.
5. **Close each issue the merge fixed** with a comment naming the merge commit and the tests
   that cover it.
6. **Clean up local state:** `git worktree remove <path>`, then `git branch -d <branch>`; check
   `git rev-list --count origin/main..<branch>` is 0 first. Remote branches delete themselves.

## The Pages deploy

- `deploy-landing.yml` publishes `landing/` to biorouter.ucsf.edu on every push to `main` that
  touches `landing/**`, `docs/website/**` or the workflow, daily for the model catalog, and on
  dispatch. It runs in its own `pages` group and is never cancelled by a push.
- "Fetching artifact metadata failed" or "No artifacts named github-pages were found" in the
  `deploy` job is a transient GitHub error (three times on 2026-11-25). Rerun the failed job:
  `gh run rerun <run-id> --failed`.
- Done means the live site shows the change: fetch the page or `registry.json` and look for the
  new row or text. A green run alone is not the check.

## Related

- `watching-ci.md` for waiting, reading failures and reruns.
- `releases.md` for what may not merge while a release is in flight.
- `docs/testing/continuous-integration.md` "A newer push to `main` cancels the older run".
