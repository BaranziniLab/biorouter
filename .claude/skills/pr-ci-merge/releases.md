# Releases, release gates and announcements

Read this when a merge, a fix or an announcement touches a release of the app, a skill or an
extension. The app's release pipeline, its phases and its invariants are in CLAUDE.md
"Releasing (cross-platform)" and `.claude/workflows/release.js`; this file covers how a release
interacts with reviews, PRs and `main`.

## Review before release

1. **A fix to a security finding gets its own independent review before any release.** On
   2026-11-25 a release agent fixed a high finding in a skill and released it in the same step.
   An independent review of the published zip then found a new high that the fix itself had
   introduced. The marketplace card was held, so nothing was advertised. A fix is new code
   written under pressure to make one case pass; it is the least reviewed code in the release.
2. **Never let one agent both fix a high or medium finding and release.** In a workflow:
   fix, then an independent review of the fixed code, looping until a review is clean, then a
   separate release agent, then an independent review of the published bytes.
3. **Give the fix's reviewers the earlier findings,** and tell them to look for new ways in that
   the fix opened, with many generated input variants rather than only the listed ones.
4. **Set a round cap and a stop rule before the loop starts,** for example "no high, and mediums
   only in documented limits". Past the cap, bring the remaining list to the owner. A review
   loop over an open-ended input space never reaches zero mediums by itself.
5. **Hold the release behind a switch the owner can flip** (a `RELEASE_AFTER_CLEAN` constant in
   the workflow script), so a clean loop can release without a new run and an unclean one cannot.
6. **A review without an explicit verdict line is a failed review.** A reviewer that died
   once returned "waiting" for 30 hours and was read as a pass.

## Advertising waits for the published bytes

- A marketplace card, a registry row, a website news row and any announcement wait until an
  independent review of the artifact as published (the zip, the `.brxt`, the installers) is
  clean. The card PR may be open and green meanwhile; it stays unmerged.
- Publishing an extension release updates every install: the desktop polls the extension
  manifest repository's latest release (`ui/desktop/src/utils/extensionUpdater.ts`). Publish
  only on the owner's word.

## The app release window

From the release build until publish, `main` must not move.

- **Nothing merges to `main` in that window,** website and marketplace PRs included. `draft`
  pins the release to the exact source commit and refuses when `origin/main` differs, and
  `publish` refuses a draft whose target is not the source commit. Queue other PRs and merge them
  after publish (the 1.94.6 marketplace card PR waited for exactly this).
- **Commit nothing in the release worktree while `scripts/release.sh` runs.** Every phase checks
  that `HEAD` still equals the `source_sha` recorded at the build; even a docs-only commit kills
  the next phase and forces a rebuild. Write and push the release notes before the build starts.
- **A release PR has auto-merge off.** Dispatch the package workflows at the release ref, build
  the macOS artifacts from that commit, and once the PR's required checks have passed on that
  SHA, land it by fast-forward so `main`, the artifacts and the tag share one commit:

  ```bash
  git push origin "${sha}:refs/heads/main"   # a fast-forward, not a force
  ```

  GitHub then marks the PR merged and deletes its branch.
- **A flaky job on the release PR is rerun, not pushed over:** `gh run rerun <run-id> --failed`
  keeps the SHA, so the artifacts built from it stay valid. A push means rebuilding every
  artifact.
- **If the source must change, rebuild everything from the new commit and draft last.** Before
  publishing, compare the draft's target with the build record:

  ```bash
  gh release view v<ver> --json targetCommitish --jq .targetCommitish
  awk -F'\t' '$1=="source_sha"{print $2}' dist/release-build-<ver>.tsv
  ```

  A draft has no tag yet, so `git ls-remote --tags` is empty and proves nothing.
- **Start the slowest gate as soon as the draft exists.** The dictation accuracy run takes hours.
  If it ends cancelled by its timeout, `gh run rerun <run-id> --failed` reruns only that job and
  keeps the run's title and commit, which is what `publish` looks for. A second dispatch on the
  same ref queues behind the first; for parallel copies use throwaway `gate/<name>-<n>` branches
  at the source commit (`watching-ci.md`).
- **Publishing is two steps.** `draft`, then the native smoke gates, then `publish`. Never
  replace them with a hand-written `gh release create`, which skips the gates.
- **After publish:** the website news rows and `scripts/release.sh landing <ver>`, then the
  held PRs, then the umbrella issue, then the release branches.

## Skill and extension releases

- Skills live in the separate skills repository and release through its own release script and
  verify step, with a digest commit recording the published asset's hash. Review the published
  zip independently, then open the marketplace card PR in this repo (catalog sync, card,
  `node landing/scripts/build-registry.mjs`, and its checks).
- Extensions publish a `.brxt` per release from their own repositories. Each new extension gets a
  security review before its first release, and the same review-the-fix rule applies to every
  release after.
- Do not cut a skill or extension release from a branch that has not landed on its repository's
  `main`, and push that `main` before the release script runs.

## Related

- SKILL.md "Releases and announcements" for the short form of these rules.
- CLAUDE.md "Releasing (cross-platform)" for the phases, the eleven assets and the publish gate.
- `merging.md` for reading `main` and the Pages deploy after the release lands.
