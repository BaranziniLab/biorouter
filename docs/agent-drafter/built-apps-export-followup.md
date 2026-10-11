# Built apps export followup

Worktree: `/Users/wgu/Desktop/BioRouter/.claude/worktrees/built-apps-export-followup`  
Branch: `codex/built-apps-export-followup`  
Base: the original built-apps hardening handoff snapshot, including its interrupted export fixes.

## Findings disposition

- **Launcher re-sync could erase installed edits and leave a false bundle stamp.** The baseline had introduced runtime digests, backups and a rule retaining a daemon-derived `dist/` under a retained manifest. This pass completes that fix: install records identify the export folder that wrote them; another older folder cannot trust its record. Digests cover root assets and custom directories, while a daemon-derived SDK and bundle do not break the edit/run loop. A refresh retains the recipient's SDK, bundle and manifest, and the daemon rebuilds when authored source changes. Independently edited installed runtime files are preserved; an explicit replacement makes a backup.
- **`replace:true` destroyed the earlier export.** Already fixed in the baseline: the previous folder is renamed beside the new export and its path appears in the report. An earlier export containing project metadata or additional user files is refused. This pass also prevents a `.git`/`node_modules` in the source app from making those entries acceptable in a replacement target.
- **Shipped skills could overwrite a recipient's newer copy.** The baseline filtered shipped skill names. This pass also filters the seeded `knowledge-bases` bundle root and mixed-case aliases such as `Office-Word`, which otherwise resolve to a seeded directory on macOS. The HTTP door and desktop mirrors need the matching changes from the security and desktop workstreams.
- **Desktop user-action proof came from changed settings.** Revalidated by the desktop workstream; already fixed in the baseline. The attached daemon's proof remains authoritative.
- **Model-selected target placement was still a deny list.** The tool now allows only targets below canonical Desktop, Documents or Downloads roots. Existing protections for Biorouter stores, hidden directories, code-import directories and non-export replacement targets remain. Forbidden components are checked in the canonical content root too, preventing a Desktop link into site-packages or hidden settings from bypassing placement; ordinary redirected/cloud content folders remain allowed. Person-selected exports through the desktop folder picker keep their existing placement rules.

## Additional functional fixes from independent review

- Every launcher installs root assets and custom authored directories, including `styles.css` and `scripts/chart.js`, rather than only entry/src/dist/assets. Runtime files or entire authored subtrees removed from the export are removed from an unchanged installation. Backups include those runtime files too.
- `serve.mjs` now proxies all app assets through daemon authorization and security headers. It serves the daemon's rebuilt bundle, and a page refresh reruns the preservation check before installing edits. Concurrent refreshes are serialized.
- PowerShell sorts digest lines by UTF-8 bytes, matching Rust, Bash and Node even for same-content filenames containing supplementary Unicode characters.
- The appcheck scripts discover esbuild through the local package installation, list apps from the actual daemon rather than another developer's hardcoded files, validate raw entries without a config island, report legacy skipped binaries, safely write into an owned temporary folder, and return a failing exit status when checks fail.

## Verification performed in this lane

No Cargo build or test jobs were launched here; the coordinator owns the shared build cache and integrated suites.

- Direct Rust formatting via `/Users/wgu/.cargo/bin/rustfmt --edition 2021 --config skip_children=true` for all three changed Rust files; `git diff --check` clean.
- Generated Bash syntax and Node ESM syntax passed.
- Actual Bash and Node installer execution against isolated recipient roots passed: root CSS/custom JS copied; refreshed SDK and daemon bundle retained; another source edit synchronized; competing export folders could not roll back an installation; deleted src/assets subtrees disappeared; independent installed script edits remained untouched.
- An actual generated Node server against a temporary loopback stand-in daemon passed: rebuilt daemon bytes and response headers were proxied; unauthorized assets remained 401; independent installed page edits survived refresh; payload/export metadata was not served; an unrelated Host was 421. The exact spawned processes were stopped. This verifies proxy behavior, not actual provider inference.
- Appcheck script syntax passed. `node --test ui/desktop/scripts/appcheck/export-check.test.mjs`: 3 passed; raw static entries and valid legacy IDs were accepted, and assembled config islands and path traversals were refused.
- Added Rust regressions that execute Bash/Node installers and the Node proxy. Expanded cross-runtime digest fixtures with root assets/custom directories and Unicode filenames. Added a Windows-only regression that executes the generated PowerShell digest and compares it to Rust; it still requires Windows CI execution.

## Coordinator checks

Run the combined Agent Drafter suite after integrating the matching HTTP/desktop shipped-entry changes:

```bash
cargo fmt --all
cargo test -p biorouter-mcp --lib agent_drafter::
cargo test -p biorouter-mcp --test ui_example_apps
```

The new focused names are `recipient_install_preserves_edits_and_syncs_all_authored_runtime_files`, `powershell_runtime_digest_matches_utf8_order_for_unicode_names`, and `source_project_metadata_does_not_permit_replacing_an_edited_export`. Existing `serve_mjs_serves_only_the_app_and_only_to_its_own_host` now exercises proxy authorization, retained installed edits and rebuilt daemon bytes.

For live recipient validation, keep HOME unchanged and set `BIOROUTER_PATH_ROOT` to a fresh temporary root. All three launchers and the daemon then use `<root>/config/agent_drafter/<id>`; launcher state, records, private logs and backups live in `<root>/config/app-launcher`. Real desktop clicks, ZIP modes, real daemon rebuilds/provider connection, full lint gates, CI and remote synchronization remain the coordinator's integration work.

## Combined-suite followup

The integrated MCP suite exposed a test-fixture defect: `find_command("node")` selected the Hermit Bash shim, and using it as a shebang interpreter fails with `Exec format error` on macOS. The launcher then fell through to a real installed daemon, whose app access gate correctly returned 401 to the mock cookie. The fixture now uses `/bin/sh` to execute the selected Node program and a separate JavaScript file, and requires explicit evidence that the stand-in daemon was selected. The generated proxy regression passed with that Hermit wrapper against an isolated recipient root. The Node launcher also restores the `statSync` import used by Windows Squirrel install discovery.

Fat exports previously copied only `biorouterd`. A fresh recipient without an external compiler could run an already-current bundle, but could not reliably rebuild changed source or refresh an older SDK: the fallback refuses to replace a real esbuild bundle. The export now obtains the compiler from the selected daemon's own package layout (`bundle::packaged_esbuild`, shared with desktop siblings and Linux libexec lookup), verifies its native executable header for the current platform and architecture, and stages both binaries together under `payload/bin`. Before publishing them it runs the staged compiler's `--version` with the shared stripped environment, a five-second deadline and a required exact match to the pinned release. It never copies a Node wrapper or invokes npm/npx to manufacture a compiler. The existing daemon metadata is retained and gains an `esbuild` object with relative file, byte count and verified version. If the helper is missing, invalid or cannot be copied, the report explicitly says why this is a thin export and no partial daemon payload remains.

The backend ZIP classifier and desktop unpacker both restore executable permissions for `payload/bin/{biorouterd,biorouterd.exe,esbuild,esbuild.exe}`. Added regressions cover native helper bytes and metadata, executable modes, missing/compiler-wrapper refusal without partial binaries, another-architecture refusal, exact-version validation and its deadline, failed-probe cleanup, and ZIP extraction with absent executable modes. All 14 `appExport.test.ts` tests passed in an isolated copied-source fixture using the installed dependencies, Node 26 and one worker. Rust formatting, Prettier and diff checks passed. A small standalone `rustc` smoke compiled the actual native-header parser and accepted the coordinator's exact helper; it reported the pinned version 0.27.3. No Cargo jobs were launched in this lane.

This checkpoint depends on the narrow build-workstream commit exposing `bundle::packaged_esbuild`. The coordinator still needs to execute the new Rust tests and force source/SDK rebuilds from an extracted fat export outside a checkout, with a fresh `BIOROUTER_PATH_ROOT`, no compiler override and no Node or esbuild on PATH. Windows executable digest and helper extraction require Windows CI.

The subsequent full Rust suite found that the older public-tool export fixture supplied only a dummy daemon. Its test is now named `export_bundle_daemon_current_checks_packaged_compiler_contract`. It copies a real compiler from the explicit QA hook or this checkout's native packaged helper, then requires both binaries byte-for-byte, verified compiler metadata and the fat-export report. Rust-only CI installs Node but has no npm dependency install or native helper staging before library tests; in that case the test explicitly requires the missing-compiler thin report, absent binaries and absent bundled metadata. A separate unconditional public Full-export test, `export_bundle_daemon_missing_packaged_compiler_is_explicitly_thin`, verifies that the same condition yields null bundled metadata, no partial bin directory and a precise note. Compiler discovery never uses a JavaScript `.bin` wrapper, another checkout or npx. Formatting and diff checks passed; execution of these public-path regressions remains with the coordinator.
