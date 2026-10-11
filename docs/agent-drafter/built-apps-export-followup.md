# Built apps export followup

Worktree: `/Users/wgu/Desktop/BioRouter/.claude/worktrees/built-apps-export-followup`  
Branch: `codex/built-apps-export-followup`  
Base: the original built-apps hardening handoff snapshot, including its interrupted export fixes.

## Findings disposition

- **Launcher re-sync could erase installed edits and leave a false bundle stamp.** The baseline had introduced runtime digests, backups and a rule retaining a daemon-derived `dist/` under a retained manifest. This pass completes that fix: install records identify the export folder that wrote them; another older folder cannot trust its record. Digests cover root assets and custom directories, while a daemon-derived SDK and bundle do not break the edit/run loop. A refresh retains the recipient's SDK, bundle and manifest, and the daemon rebuilds when authored source changes. Independently edited installed runtime files are preserved; an explicit replacement makes a backup.
- **`replace:true` destroyed the earlier export.** Already fixed in the baseline: the previous folder is renamed beside the new export and its path appears in the report. An earlier export containing project metadata or additional user files is refused. This pass also prevents a `.git`/`node_modules` in the source app from making those entries acceptable in a replacement target.
- **Shipped skills could overwrite a recipient's newer copy.** The baseline filtered shipped skill names. This pass also filters the seeded `knowledge-bases` bundle root and mixed-case aliases such as `Office-Word`, which otherwise resolve to a seeded directory on macOS. The HTTP door and desktop mirrors need the matching changes from the security and desktop workstreams.
- **Desktop user-action proof came from changed settings.** Revalidated by the desktop workstream; already fixed in the baseline. The attached daemon's proof remains authoritative.
- **Model-selected target placement was still a deny list.** The tool now allows only targets below canonical Desktop, Documents or Downloads roots. Existing protections for Biorouter stores, hidden directories, code-import directories and non-export replacement targets remain. Person-selected exports through the desktop folder picker keep their existing placement rules.

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
- Appcheck script syntax passed. Mocked daemon checks accepted a raw static entry and refused an assembled config island and a path traversal.
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
