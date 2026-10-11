# Built-app build and serve followup

The build lane continues commit `a423670d5` in the isolated worktree
`/Users/wgu/Desktop/BioRouter/.claude/worktrees/built-apps-build-followup`, branch
`codex/built-apps-build-followup`. The seven findings in
`fix-input-build-serve.json` were independently checked against the source.
The interrupted pass had already included their fixes in the handoff commit.

| Finding | Verified implementation and regression |
| --- | --- |
| CSS lost under temporary names | esbuild writes into an isolated staging directory; app.css is installed, obsolete CSS removed, staging deleted. Tests cover real CSS imports, removal, and incomplete output preserving previous JS/CSS. |
| One-build SDK downgrade protection | Bundle banners carry separate sdkv provenance. A successful stubbed build retaining a newer SDK stays fresh after another rebuild. |
| SDK import variants | Shared import predicate recognizes ./sdk, ./sdk.ts, ./sdk.js, multiline imports and reexports. Existing vendored SDKs refresh, and extension imports stamp actual vendored fingerprints. Fallback extension imports include the SDK. |
| Failed fallback mutates the SDK | Refusals happen before refresh; unsuccessful attempted builds restore the prior SDK. Launch warns while serving an existing bundle, and refuses only when no bundle exists. |
| Empty list kind | Blank and whitespace-only optional list filters mean no filter. Creation continues rejecting an explicitly blank kind. |
| unsafe-hashes handler gadget | No handler hashes or unsafe-hashes are sent. Authored handlers become once-bound listeners in a hash-pinned generated script. A real browser verifies that runtime markup cannot reuse their bodies or markers. |
| Predictable npx cache | A randomly named process cache is created with mode 0700; its owning uid and permissions are tested. Cache creation failure prevents npx execution. |

Additional changes:

- CLI-only Linux packages stage the same digest-verified pinned esbuild as the
  desktop under `/usr/libexec/biorouter/esbuild`. The daemon prefers that helper
  over a separately installed system esbuild, PATH and npx. Package smoke tests
  check its exact version and compile a fixture in clean Debian and Rocky Linux.
- Removed unused `agent.js`, `resize.js` and `starter.html` templates after
  checking production and test references. The old preview test exercised the
  retired iframe bridge; its replacement starts a private temporary daemon and
  verifies the actual served page and response policy in Chrome.

Verification performed in this lane:

- `node --test ui/desktop/scripts/fetch-esbuild.test.cjs`: **6 passed**.
- `bash -n scripts/build-cli-linux-packages.sh`: passed.
- `node --check scripts/agent-drafter/preview-runtime-test.mjs`: passed.
- Browser harness with the handoff worktree's `target/debug/biorouterd`
  (binary timestamp October 10, 2026, 16:15 local): **12 assertions passed**.
  This verifies daemon-origin launch, actual script hashes, absence of
  unsafe-inline/unsafe-hashes in script-src, authored script/onload/onclick,
  form scope, return-false navigation cancellation, marker removal, blocked
  injected onerror, blocked binder replay and blocked cloned template handlers.
  The test uses a synthetic secret, no model, a temporary profile and a single
  Chrome instance; it terminates its exact daemon child and removes its profile.

Rust builds/tests and final formatter/linter execution are serialized by the
coordinator because the shared Mac has heavy unrelated load and active swap.
The complete deb/rpm Docker build was not executed in this lane. Linux helper
discovery is covered by a Linux-only Rust regression and the packaging harness;
the final Linux CI/package jobs must execute that platform-specific path.

To repeat the browser check after the integrated daemon is rebuilt:

```bash
BIOROUTER_TEST_BIN=/absolute/path/to/target/debug/biorouterd \
  node scripts/agent-drafter/preview-runtime-test.mjs
```

Set `BIOROUTER_PLAYWRIGHT_MODULE` to an existing Playwright `index.js` when the
isolated worktree has no Node dependency installation. Set `PW_EXEC` only when
using a Chromium executable other than the system Google Chrome.

## Windows digest CI followup

The early actual-template check exposed the Windows PowerShell 5.1 failure:
`Get-FileHash` raised `CommandNotFoundException`, which the production digest
correctly handled as an unknown digest but the earlier fixture hid as an empty
string. The module search environment may explain the missing command; that
environment explanation has not been independently proven and no module path
or global installation was changed.

The Windows launcher now hashes each file through .NET SHA-256 and `File.OpenRead`,
disposes every stream in a `finally`, and disposes its hash instance after use.
The UTF-8 byte ordering and digest protocol are unchanged. Both the Rust Windows
fixture and the early Node runner retain strict equality against independently
computed digests and expose the actual exception in their test probes.

The early runner now checks the actual template with and without a throwing
`Get-FileHash` shadow; speculative array/delegate variants were removed. It also
opens hashed fixtures with exclusive access after hashing to check stream closure.
The official portable PowerShell 7.6.6 arm64 archive was checked against its GitHub
release SHA-256 and used only under `/private/tmp`, without a global installation.
All four generated-script cases passed there, including the original CI fixture
hashes with and without `src/main.ts`. Node syntax, Prettier, direct Rust formatting
and diff checks passed. No Cargo jobs ran in this lane. Windows PowerShell 5.1 CI
must still confirm the final production fix.
