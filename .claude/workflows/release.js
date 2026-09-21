/* global agent, phase, log, args */
// BioRouter release workflow — orchestrates a signed, notarized, multi-platform
// release using agents over scripts/release.sh. Runs in the Workflow sandbox,
// which provides the globals declared above (not standalone Node), so a plain
// ESLint pass would otherwise flag them as no-undef.
//
// Run it with the Workflow tool:
//   Workflow({ name: 'release', args: { version: '1.80.1' } })
// or, ad-hoc:
//   Workflow({ scriptPath: '.claude/workflows/release.js', args: { version: '1.80.1' } })
//
// Each step delegates to one focused agent that runs a single
// `scripts/release.sh <phase> <version>` step (or, for the two GitHub Actions
// gates, dispatches one workflow and waits for it) and reports a structured
// verdict, so the workflow can stop early on the first failure and you can
// resume from any phase. The heavy builds are necessarily serial (every bundle
// rewrites ui/desktop/src/bin and the cross builds share the cargo target lock),
// so the package phase runs one platform at a time on purpose.
//
// Order: bump → backends → mac-arm64 → mac-intel → CI builds (Linux + Windows)
// → adopt-ci → mac-manifest → verify → draft → native Windows smoke → publish
// → landing. The CI builds and the smoke are GitHub workflows, not release.sh
// phases; each is its own step so a red run stops the release before the phase
// that would consume it.
//
// ⚠ Every `scripts/release.sh <phase>` named below must be an arm of the
// dispatcher's `case "$CMD" in` at the bottom of release.sh — an unknown phase
// dies with the usage line. This file went on calling the local `windows`,
// `linux` and `cli-linux` packaging phases after those assets moved to CI, then
// a `headless-linux` phase the dispatcher does not have at all, and all four
// sat after the two notarization steps. It also never ran adopt-ci or landing.

export const meta = {
  name: 'release',
  description: 'Cut a signed, notarized, multi-platform BioRouter release (bump → build → notarize → publish)',
  whenToUse: 'When you want to ship a new BioRouter version end-to-end. Pass { version: "x.y.z" }.',
  phases: [
    { title: 'Prep', detail: 'bump version + write release notes + commit' },
    { title: 'Backends', detail: 'compile mac arm64/x64 + windows + linux release binaries' },
    { title: 'Package', detail: 'sign+notarize the arm64 and Intel dmgs and their updater zips' },
    { title: 'CI', detail: 'push, build Linux + Windows packages in GitHub Actions at the release commit, adopt them' },
    { title: 'Verify', detail: 'generate latest-mac.yml, then check arch, notarization, asset set, and source provenance' },
    { title: 'Draft', detail: 'prove exact remote equality, then create the 11-asset draft' },
    { title: 'Publish', detail: 'native windows smoke on the draft, uploaded digests, then flip the draft live' },
    { title: 'Landing', detail: 'point the public site at the published release, commit landing/, push' },
  ],
}

const VERDICT = {
  type: 'object',
  additionalProperties: false,
  required: ['ok', 'detail'],
  properties: {
    ok: { type: 'boolean', description: 'true only if the step fully succeeded' },
    detail: { type: 'string', description: 'one-paragraph result: artifact paths, sizes, or the error tail' },
  },
}

const version = (args && (args.version || args.v)) || (typeof args === 'string' ? args : null)
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error('release workflow needs a semver version, e.g. Workflow({ name: "release", args: { version: "1.80.1" } })')
}

// Run one release.sh phase in a dedicated agent and fail fast on a bad verdict.
async function step(phase, instructions) {
  const res = await agent(
    `You are running one phase of the BioRouter release for version ${version}, from the repo root ` +
    `/Users/wgu/Desktop/BioRouter. ${instructions}\n\n` +
    `Run the command, stream nothing back except what you need, and return a verdict. ` +
    `Treat a non-zero exit, a missing artifact, "MISSING", "NOT stapled", "WRONG ARCH", or any link/compile error as ok=false. ` +
    `Notarization and docker builds can take 10-20 minutes each, and a GitHub Actions run well over an hour — wait for them.`,
    { label: phase, phase: metaTitle(phase), schema: VERDICT },
  )
  if (!res || !res.ok) {
    throw new Error(`[${phase}] failed: ${res ? res.detail : 'agent returned null'}`)
  }
  log(`✓ ${phase}: ${res.detail.slice(0, 200)}`)
  return res
}

// Which progress group each step's agent is shown under. A lookup that throws
// rather than a fallback: the fallback used to be 'Package', which put `draft`
// in the Package group, and would have put every step added since there too.
const STEP_GROUP = {
  'prep': 'Prep',
  'backends': 'Backends',
  'mac-arm64': 'Package',
  'mac-intel': 'Package',
  'ci-builds': 'CI',
  'adopt-ci': 'CI',
  'mac-manifest': 'Verify',
  'verify': 'Verify',
  'draft': 'Draft',
  'windows-smoke': 'Publish',
  'publish': 'Publish',
  'landing': 'Landing',
}

function metaTitle(phase) {
  const title = STEP_GROUP[phase]
  if (!title) throw new Error(`release workflow: step '${phase}' has no progress group in STEP_GROUP`)
  return title
}

phase('Prep')
await step('prep',
  `Run \`bash scripts/release.sh bump ${version}\` to bump all 6 version-bearing files in lockstep. ` +
  `Then write concise patch/minor release notes to docs/releases/notes/v${version}.md based on \`git log <previous-tag>..HEAD\`, ` +
  `modelling the format on the newest existing docs/releases/notes/*.md by version (\`ls docs/releases/notes | sort -V | tail -1\`). ` +
  `Then commit ONLY the 6 version-bearing files + the new release notes with message ` +
  `"release v${version}". Do not add Co-Authored-By, AI-generated, or other automated attribution trailers; the commit policy rejects them. ` +
  `Do NOT commit unrelated working-tree changes.`)

phase('Backends')
await step('backends',
  `Run \`bash scripts/release.sh backends ${version}\`. This compiles the release backend for mac arm64, mac x64, ` +
  `windows-gnu (docker), and linux-gnu (docker), applying the winpthread + LZMA_API_STATIC cross-compile fixes. ` +
  `Confirm target/release, target/x86_64-apple-darwin/release, target/x86_64-pc-windows-gnu/release (.exe + 3 dlls), ` +
  `and target/x86_64-unknown-linux-gnu/release all hold fresh binaries.`)

// Package phase — macOS only, STRICTLY serial (each bundle clobbers
// ui/desktop/src/bin). The Linux and Windows packages are no longer built on
// this machine; the next phase takes them from CI.
phase('Package')
await step('mac-arm64', `Run \`bash scripts/release.sh mac-arm64 ${version}\` — signs + notarizes the Apple Silicon dmg. Verify ui/desktop/out/make/Biorouter-${version}-arm64.dmg exists and the app reports "Notarized Developer ID".`)
await step('mac-intel', `Run \`bash scripts/release.sh mac-intel ${version}\` — signs + notarizes the Intel dmg. Verify ui/desktop/out/make/Biorouter-${version}-x64.dmg exists and its bundled binary is x86_64.`)

// The six Linux and Windows assets come from GitHub Actions: the local Docker
// Linux package left container-written files `--w-------` on a macOS host, and a
// Windows zip and installer built in two places carried different backends. The
// measured detail is above `ci_run_for_release` in scripts/release.sh. adopt-ci
// only accepts a successful run whose headSha is the manifest's source_sha, so
// the push and the dispatch have to happen first, from exactly that commit.
phase('CI')
await step('ci-builds',
  `The Linux GUI deb + rpm, the CLI-only deb + rpm, the Windows zip and Biorouter-Setup-${version}.exe are built by ` +
  `GitHub Actions, not on this machine, and the next step only accepts them from successful runs whose head commit is ` +
  `this release's source commit. ` +
  `(1) Read the source commit: \`awk -F'\\t' '$1=="source_sha"{print $2}' dist/release-build-${version}.tsv\`, and confirm it equals \`git rev-parse HEAD\`. ` +
  `(2) Push it: \`git push origin main\`, then \`git fetch origin main\` and confirm \`git rev-parse origin/main\` equals the source commit. ` +
  `A dispatch builds whatever origin/main points at when it lands, because the dispatch API takes a branch or tag name, not a commit. ` +
  `(3) Dispatch both: \`gh workflow run linux-gui-packages.yml -f version=${version} --ref main\` and ` +
  `\`gh workflow run windows-gui-packages.yml -f version=${version} --ref main\`. ` +
  `(4) For each workflow, find the run your dispatch created in ` +
  `\`gh run list --workflow <file> --event workflow_dispatch --limit 10 --json databaseId,headSha,status,conclusion,createdAt\` ` +
  `(it can take a few seconds to appear; an older run is not it), ` +
  `and wait for it with \`gh run watch <id> --exit-status\`, re-issuing the watch as often as your shell timeout requires. ` +
  `ok=true only when BOTH runs completed with conclusion "success" AND a headSha equal to the source commit. ` +
  `A run at any other headSha means main moved after the push: report ok=false rather than dispatching again. ` +
  `For a failed run, include the tail of \`gh run view <id> --log-failed\`. Do not fall back to packaging Linux or Windows locally.`)
await step('adopt-ci',
  `Run \`bash scripts/release.sh adopt-ci ${version}\`. For each of linux-gui-packages.yml and windows-gui-packages.yml it ` +
  `selects a successful run whose headSha is the manifest's source_sha, downloads that run's artifact, and records six files ` +
  `in the provenance manifest. Confirm dist/release-build-${version}.tsv now has exactly one \`ci_run\` row for each ` +
  `workflow (\`awk -F'\\t' '$1=="ci_run"' dist/release-build-${version}.tsv\`) and that these six files exist: ` +
  `ui/desktop/out/make/deb/x64/biorouter_${version}_amd64.deb, ui/desktop/out/make/rpm/x64/Biorouter-${version}-1.x86_64.rpm, ` +
  `dist/cli/biorouter-cli_${version}_amd64.deb, dist/cli/biorouter-cli-${version}-1.x86_64.rpm, ` +
  `ui/desktop/out/make/zip/win32/x64/Biorouter-win32-x64-${version}.zip and ` +
  `ui/desktop/out/make/squirrel.windows/x64/Biorouter-Setup-${version}.exe. ` +
  `If it dies with "no successful … run", the previous step's runs are not at the source commit: report ok=false and do not package locally.`)

// mac-manifest runs before verify, not only inside draft: verify requires exactly
// one provenance row for each of the 11 assets, latest-mac.yml included, and only
// mac-manifest records that row.
//
// verify no longer starts with `rm -rf node_modules && npm ci`. That preamble
// existed because the local Linux docker package left node_modules
// Linux-flavored, and nothing in this sequence packages Linux locally any more.
phase('Verify')
await step('mac-manifest',
  `Run \`bash scripts/release.sh mac-manifest ${version}\`. It regenerates ui/desktop/out/make/latest-mac.yml from ` +
  `Biorouter-darwin-arm64-${version}.zip and Biorouter-darwin-x64-${version}.zip and records it in the provenance manifest. ` +
  `Confirm the file exists and names both zips.`)
await step('verify',
  `Run \`bash scripts/release.sh verify ${version}\`. It must accept all 11 local assets against the durable ` +
  `dist/release-build-${version}.tsv provenance manifest; a missing manifest, changed source SHA, dirty source tree, stale artifact, ` +
  `digest mismatch, duplicate, or extra asset is a hard failure. ` +
  `The 11 assets are 6 GUI (Biorouter-${version}-arm64.dmg, Biorouter-${version}-x64.dmg, Biorouter-win32-x64-${version}.zip, ` +
  `Biorouter-Setup-${version}.exe, biorouter_${version}_amd64.deb, Biorouter-${version}-1.x86_64.rpm), 2 CLI-only Linux ` +
  `(biorouter-cli_${version}_amd64.deb, biorouter-cli-${version}-1.x86_64.rpm) and 3 macOS auto-update artifacts ` +
  `(Biorouter-darwin-arm64-${version}.zip, Biorouter-darwin-x64-${version}.zip, latest-mac.yml). Without the last three, ` +
  `macOS clients 404 on the in-app updater and fall back to an assisted download.`)

phase('Draft')
await step('draft',
  `The release commit was pushed in the CI step. Run \`bash scripts/release.sh draft ${version}\`: \`cmd_draft\` requires ` +
  `a successful fetch and exact \`HEAD == origin/main\`, then targets that immutable commit SHA, regenerates latest-mac.yml, ` +
  `asserts all 11 assets exist, and creates the DRAFT release. It deliberately stops there. If it refuses because HEAD and ` +
  `origin/main differ, report ok=false: do not push, pull or rebase, because a moved HEAD invalidates every artifact built so far.`)

// Publication is gated on a native Windows smoke run, because nothing earlier in
// this pipeline executes the Windows build on Windows. The smoke downloads the
// zip from the DRAFT, so it can only run after draft, and it is its own step so
// a red smoke stops the workflow before publish is attempted.
phase('Publish')
await step('windows-smoke',
  `Dispatch the native Windows smoke against the draft: \`gh workflow run release-artifact-smoke.yml -f version=${version} --ref main\`. ` +
  `Find the run your dispatch created in \`gh run list --workflow release-artifact-smoke.yml --limit 10 --json databaseId,displayTitle,headSha,status,conclusion,createdAt\` ` +
  `(displayTitle "Release artifact smoke v${version}"; it can take a few seconds to appear, and an older run is not it: ` +
  `publish only accepts a smoke that started after the newest draft asset upload), and wait for it with \`gh run watch <id> --exit-status\`. ` +
  `ok=true only when it completed with conclusion "success" and its headSha equals the source_sha in dist/release-build-${version}.tsv. ` +
  `For a failed run, include the tail of \`gh run view <id> --log-failed\`.`)
await step('publish',
  `Run \`bash scripts/release.sh publish ${version}\`. It re-runs verify, compares all 11 ` +
  `uploaded GitHub SHA-256 digests and sizes to the local provenance-bound files, and requires a successful smoke for the same ` +
  `source SHA whose start time is later than the newest draft asset upload. Any replaced asset therefore requires a new smoke run. ` +
  `Finally confirm \`gh release view v${version}\` shows exactly 11 uploaded assets and is not a draft.`)

// After publish, deliberately: the site's hardcoded versions are fallbacks for
// when GitHub is unreachable, so they must name a published release, and
// cmd_landing refuses a draft.
//
// ⚠ Two News entries are hand-written here, not one. The News lists are HISTORY
// (every older row names an older release), so cmd_landing deliberately does not
// rewrite them — a blanket version replace once relabelled content.md's 1.90.5
// entry as 1.91.0. Instead its pre-flight REFUSES, before editing anything,
// unless BOTH landing/about.html's .news-list AND landing-site-content.md's
// '### News' section already link v<ver>. An earlier version of this step wrote
// only the about.html row, so it could never pass that pre-flight.
phase('Landing')
await step('landing',
  `landing/scripts/check-consistency.mjs requires landing/about.html to link releases/tag/v${version}, and no script ` +
  `writes that file. So FIRST run \`grep -F 'releases/tag/v${version}"' landing/about.html\`. If it has no match, add a ` +
  `news row for v${version} directly above the newest \`class="news-row"\` entry, copying that entry's markup exactly: the ` +
  `publication day and month from \`gh release view v${version} --json publishedAt\`, an <h3> "Biorouter v${version}: <short headline>", ` +
  `and one <p> summarising docs/releases/notes/v${version}.md. ` +
  `SECOND, run \`grep -F 'releases/tag/v${version}' landing/assets/landing-site-content.md\`. If the '### News' section has no ` +
  `match, add \`1. **Biorouter v${version} Release**\` as its first entry, with its Link to ` +
  `https://github.com/BaranziniLab/biorouter/releases/tag/v${version} and a What's new line from the notes, copying the shape ` +
  `of the entry below it, then renumber the entries beneath. Do NOT edit any existing News entry: they are history. ` +
  `Then run \`bash scripts/release.sh landing ${version}\`, which refuses unless both rows exist, then points the site's other pages at v${version} and runs the ` +
  `site's consistency checks. If it fails, report ok=false with the checker's output and commit nothing. If it passes, ` +
  `stage only landing/ (\`git add landing\`), confirm \`git diff --cached --name-only\` lists nothing outside landing/, commit with ` +
  `message "landing: cite v${version}" (no Co-Authored-By, AI-generated, or other automated attribution trailers; the commit ` +
  `policy rejects them), and \`git push origin main\`. That push triggers deploy-landing.yml: wait for its run and report ` +
  `ok=false if it does not succeed.`)

// (A top-level `return` here is valid inside the Workflow async-wrapper runtime
// but a fatal parse error to a plain ESLint pass — the workflow's completion is
// reported via this log line instead.)
log(`release workflow complete: v${version} released`)
