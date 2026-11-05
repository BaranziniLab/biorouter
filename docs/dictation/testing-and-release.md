# Dictation testing and release

> **What this is.** How local dictation is proved and shipped: the tests for each component and surface, the CI jobs that run them and the network each one may use, the release guards and packaging checks, the committed fixtures and their size budget, the real speech accuracy gate, the manual checklist for every surface, and the measurements still to take.
> **Status:** Current: a plan; nothing in it is implemented yet
> **Audience:** developers who write each pull request's tests and CI changes; the maintainer who cuts releases and runs the manual checklist; reviewers who check that a pull request carries its own gates.

The [implementation plan](implementation-plan.md) decides what is built and in which order. This document decides how each piece is proved before it merges, before a release is published, and before version 1 is announced. Almost every test runs offline inside CI steps that exist today, with whisper.cpp's own empty test models and one committed speech clip. What CI cannot do (a real microphone, a macOS permission prompt, Safari, a person's terminal, a real proxy, a signed app on macOS 12) goes into a manual checklist with one section per surface. Accuracy is held by two tiers: a networked nightly job on public real speech, and a maintainer run on a private lab recording set before any release that changes the engine or a model pin. No accuracy claim about the product goes into user documentation, release notes, the announcement or the interface until that lab baseline exists.

## How to read this document

Load bearing claims carry the plan's labels:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, browsers or audio |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main` |
| [R] | Reported by a third party's source, documentation or issue tracker, not measured here |
| [I] | Inferred; where it matters, the measurement that would settle it is named |

Identifiers follow the plan: R1 to R8 (the owner's requirements), S1 to S20 (surfaces), DD-1 onward (the [decision register](implementation-plan.md#decision-register), which is binding here), PR-01 to PR-33 and A-1 (pull requests and the owner's setup action), O-1 onward (owner decisions), MSG-… (message ids, whose exact text is in [surfaces](surfaces.md)), P-1 to P-15 (privacy invariants) and M-1 to M-21 (measurements still to take). This document elaborates DD-3, DD-9 to DD-13, DD-17, DD-44, DD-50, the gate of DD-57, DD-94 and DD-95, every privacy invariant and every measurement.

Conventions used below:

| Term | Meaning |
|---|---|
| lib | `rust.yml` job `test`, step "cargo test (workspace, lib + bins)": `--lib --bins` on ubuntu, macOS and Windows (MSVC) |
| offline | `rust.yml` job `test`, step "cargo test (integration binaries, no network)": every `tests/*.rs` target of the workspace except those its exclusion table names (today `sandbox`, `ollama_readiness`, `ollama_diagnostics_20261006_2`, `ollama_protocols` and `ui_example_apps`), ubuntu only, inside `sudo unshare --net` with only loopback up [C] |
| vitest | `frontend.yml` job `unit` |
| `tests/<name>.rs` | an integration binary in the crate named beside it; its target name is `<name>` |
| a link | a file that exists at `main`; files the plan creates are named in code spans |

This document gives no count of tests. A pull request that asserts "the previous count plus N" measures both numbers in its own run, as `CLAUDE.md` asks of every count it carries.

## Principles

1. **Offline by default.** Every new `tests/*.rs` binary runs in the offline step, which builds every integration target its exclusion table does not name, runs it with `CARGO_NET_OFFLINE=true` and the keyring disabled, and never passes `--ignored` [C `rust.yml`]. No dictation binary gets a line in that step's exclusion table. A test that needs a real speech model or the network is `#[ignore = "downloads a pinned speech model"]` inside the same binary, the pattern of [`llamacpp_integration.rs`](../../crates/biorouter/tests/llamacpp_integration.rs) [C]. The offline step compiles it and skips it; `dictation-accuracy.yml` or a maintainer runs it.
2. **Unique target names.** The offline step fails when two packages share an integration target name [C]. Every binary named here is unique across the workspace.
3. **Gates land with the change.** The guard, smoke line or CI step that proves a pull request lands in that pull request (rule 2 of the plan's delivery rules). No pull request job links the Linux or Windows release binaries: `cross-check` runs `cargo check`, which runs build scripts and so compiles the C and C++ in both cross images, but never links [C]. A pull request that touches the engine, its build or `vendor/whisper.cpp/` therefore dispatches `rust.yml` before merge and links the green run in its description; the dispatch also runs `cross-build-nightly` [C].
4. **Keyless cases in their own binaries.** The user action digest is a process global `OnceLock`, so a binary that installs it can never observe a keyless daemon again. Every keyless case lives in a `*_no_user_key.rs` binary, as [`turn_control_no_user_key.rs`](../../crates/biorouter-server/tests/turn_control_no_user_key.rs) does today [C].
5. **Sandboxed.** Every new `crates/biorouter-server/tests/*.rs` declares `#[path = "../src/test_sandbox.rs"] mod test_sandbox;`, which [`every_test_binary_is_sandboxed.rs`](../../crates/biorouter-server/tests/every_test_binary_is_sandboxed.rs) enforces [C]. A CLI binary that spawns `CARGO_BIN_EXE_biorouter` sets `BIOROUTER_PATH_ROOT` to a temporary folder for the child, because the CLI's own sandbox covers its library tests, not a child process [I]. No test reads the person's real configuration, data folder, keychain or microphone.
6. **CPU only in CI.** Engine tests set `use_gpu = false`. Whether hosted macOS arm64 runners expose Metal is unmeasured [I]. Metal is proved by `smoke_mac`, Tier 2 and the checklist.
7. **Tests pin the measured side.** Where the research disagreed with itself, the tests encode what was measured on real binaries: timestamps on (small.en collapsed on 12 of 49 real utterances with them off and 2 of 49 with them on [M]); the voice detector always on, including on the retry; greedy decoding; the prompt as `prompt_tokens`, never `initial_prompt`; and no Electron test ever passes `--use-fake-ui-for-media-stream`, because with it Electron skips the permission handler entirely (zero handler calls; audio and video granted under a deny all policy) [M].
8. **No accuracy claim before the lab baseline** (DD-95). Before the lab baseline, no accuracy claim about the product appears in user documentation, release notes, the announcement or the interface, and synthetic speech never supports one. Design measurements in `docs/dictation/` and `docs/research/` stay, labeled [M] with their date. After the baseline, only dated tables.
9. **What "works when it appears" means for tests** (DD-94). A control, command or route merges only with a test that drives it end to end on the surfaces it reaches: an integration binary for a route, a Playwright run in the dev app or against `serve` for a control, a real model `#[ignore]` run recorded in the pull request for transcription quality, or a recorded checklist row for what only a person can do. Controls that PR-18 to PR-20 hide on `isBrowserSurface()` carry a vitest that pins them hidden there; PR-21 replaces that test with the `serve` job's browser run.

## Test plan per component

Each subsection lists the tests that prove one component, what each asserts, the pull request that adds it and where it runs. The subsections follow the components; the index below maps each pull request to its proof.

### Index by pull request

| PR | Proof that lands with it | Runs in |
|---|---|---|
| PR-01 | `tests/no_cloud_audio_routes.rs` (`biorouter-server`); exclusion lists removed from `settingsVocabulary.test.ts`, `emphasisWeight.sourceGuard.test.ts`, `typeScale.test.ts` | offline, vitest |
| PR-02 | `scripts/check-macos-deployment.sh` and its failing fixture | `test` macOS leg, `release.sh` |
| PR-03 | header unit tests in `routes/apps.rs` and `routes/web_ui.rs`; Playwright Chromium app page test | lib, `serve` |
| PR-04 | `styles/forgeMacUsage.test.ts`; `permissionPolicy.test.ts` frame cases; the `UserMessage.tsx` log grep test; entitlement and usage string checks | vitest, `release.sh verify`, `smoke_mac` |
| PR-05 | `managed::` table tests; parse isolation; `call_tool` managed deny | lib |
| PR-06 | a new row in [`path_resolver_agreement.rs`](../../crates/biorouter/tests/path_resolver_agreement.rs) | offline |
| PR-07 | Copilot contract tests for Biorouter's own windows and the chords | lib |
| PR-08 | `scripts/check-vendored-whisper-cpp.sh`, `scripts/check-dictation-fixtures.sh` and their self tests; the ancestry check record | `guards` |
| PR-09 | leaf crate lib tests; `tests/dictation_engine_offline.rs`; the self test in every smoke; `scripts/check-dictation-isa.sh`; `scripts/check-dictation-build-prereqs.sh`; the runtime deps named refusal; `windows-native-dictation` | lib, offline, `guards`, `cross-build-nightly`, packaging |
| PR-10 | supervisor tests with a fake worker; `tests/dictation_worker.rs` (`biorouter-cli`); `status --format json` schema checks in the package smokes | lib, offline, packaging |
| A-1 | each release immutable; `gh release verify-asset`; digests equal the pins; the evaluation asset published before PR-12 | by hand, then `dictation-model-pins.yml` |
| PR-11 | catalog invariants; installer against a loopback server; `tests/dictation_cli.rs` (`biorouter-cli`, created here) with the model commands; `tests/model_download_live.rs` (`#[ignore]`); the R6 packaging guards (the ggml magic scan, `no_model_is_embedded`, the binary size ceiling); `dictation-model-pins.yml` | lib, offline, packaging, nightly |
| PR-12 | decode tests; filter and parameter tables; `transcribe` added to `dictation_cli.rs`; `tests/dictation_accuracy.rs` (`biorouter`); `dictation-accuracy.yml` and the publish gate | lib, offline, Tier 1 |
| PR-13 | store tables; CLI dictionary tests; the store inspector; the tier proof walk | lib, offline |
| PR-14 | BPE, prompt, ranking, key, correction, decoder bonus and bias set tables; `eligible_entries_are_built_only_by_bias_set`; the census row for `visible_to` | lib, offline, Tier 1 |
| PR-15 | `tests/dictation_routes.rs`, `dictation_routes_auth.rs`, `dictation_keyed.rs`, `dictation_no_user_key.rs`, `dictation_policy_routes.rs`; OpenAPI check | offline |
| PR-16 | `tests/dictionary_routes.rs`, `dictionary_routes_auth.rs`, `dictionary_no_user_key.rs`, `dictionary_oracles.rs`; census rows | offline |
| PR-17 | transport, store, error map and insert tests; source guards; the `dictation-capture-electron` job | vitest, Electron job |
| PR-18 | `ModelsSection.test.tsx`, `settingsSections.test.ts`, `speechModelStore.test.ts`, `modelsGeometry.browser.test.tsx` | vitest |
| PR-19 | `DictationButton.test.tsx`, `DictationButton.browser.test.tsx`, `ChatInput.dictation.test.tsx`, structure and geometry tests; the dev app Playwright run for one ⌘Z | vitest, by hand |
| PR-20 | `DictionaryView.test.tsx`, `useDictionary.test.ts`, `parseTermList.test.ts`, `dictionaryRows.browser.test.tsx`; the landing consistency check; menu label and `{text, x, y}` payload tests; the **Edit your dictionary** links in the setup popover and Settings | vitest |
| PR-21 | Playwright Chromium and Firefox capture, nginx prefix and 413 halving, worklet build output check, banner snapshots | `serve`, lib |
| PR-22 | `recognize/learn.rs` golden pairs; the browser test for stale pairs | lib, vitest |
| PR-23 | capability tests; `PLATFORM_EXTENSIONS` count 7 to 8; `capabilities.test.ts`, `toolGlyph.test.ts` | lib, offline, vitest |
| PR-24 | extractor tests through `Agent::reply`; the M-21 precision run | lib, by hand |
| PR-25 | capture child, controller, environment, message, exit code and clipboard helper tests; `capture_consent_is_minted_in_one_file`; the daemon stays free of `biorouter-dictation-capture`; the P-10 check on the release build in `cross-build-nightly`; real microphone rows | lib, offline, `guards`, `cross-build-nightly`, checklist |
| PR-26 | `App` unit tests, `draw_input` snapshots, pseudo terminal test | lib, offline |
| PR-27 | Crew composer tests; `scripts/check-crew-manual.mjs` | vitest |
| PR-28 | `dictationAnywhere.test.ts` state machine; preview insert; Copilot chord extension | vitest, lib, M-12, M-15 |
| PR-29 | external backend cases; the managed policy path table test | vitest, lib, M-20 |
| PR-30 | ACP audio note, `/dictate` command and `--ws` tests | lib |
| PR-31 | the Docker workflow (build, self test, `serve`) | Docker workflow, M-16 |
| PR-32 | `pytest-jupyter` handler tests, Jest tests, one Galata smoke | `jupyter-ai.yml`, M-13 |
| PR-33 | `scripts/verify-docs.sh`, `python3 scripts/docs-lint.py`, the retired name guard; every checklist section passes | `guards`, by hand |

### Engine and worker

The leaf crate is `crates/biorouter-dictation`; the core service is `crates/biorouter/src/dictation/service.rs`.

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| `fixtures_match_their_pins` (leaf crate lib) | each of the four fixtures hashes to its SHA-256 in [Fixtures and their size budget](#fixtures-and-their-size-budget); a CRLF or LFS pointer checkout fails here and names the file | PR-09 | lib, 3 OSes |
| `src/cpu.rs` gate table | an x86 feature set missing any of AVX2, FMA, F16C or BMI2 gives `unsupported_cpu` with `missing_cpu_features` listing it; Linux arm64 without `dotprod` is refused; `sysctl.proc_translated == 1` puts `ROSETTA_ADVERTISE_AVX=1` into the child environment on macOS 15 or later and gives MSG-CPU-ROSETTA before macOS 15 (DD-9) | PR-09 | lib |
| macOS self inspection (`#[cfg(target_os = "macos")]`) | reads its own unstripped test executable: minimum OS 12.0; `MTLResidencySetDescriptor` imported weak; no `$NEWLAPACK` symbol (DD-10 rule 6, DD-11) | PR-09 | lib, macOS leg |
| `log_filter_drops_long_lines` | a 300 byte line is dropped; an 80 byte warning is forwarded; info lines are dropped; `print_realtime`, `print_progress`, `print_timestamps` and `print_special` are false in the parameter builder (DD-13, P-12) | PR-09 | lib |
| `worker_entry_runs_first` (source test) | in `crates/biorouter-server/src/main.rs`, `biorouter_dictation::worker::run_if_invoked()` is the statement right after `biorouter::fork_safety::complete_libnotify_init()` and before `run_on_agent_stack`; in `crates/biorouter-cli/src/main.rs` the same place holds `run_if_invoked()`, and from PR-25 the `capture` role's dispatch into `crates/biorouter-dictation-capture` directly before it (DD-4, DD-5) | PR-09, PR-25 | lib |
| `tests/dictation_engine_offline.rs` | the voice detector finds at least one segment within 0 to 11 s of `jfk.wav` and none in 5 s of generated silence or seeded pink noise; `whisper_full` with `for-tests-ggml-base.en.bin` returns 0 with empty text [M]; a corrupt model header generated in the test is refused before any large allocation (the CVE-2026-110810 class) | PR-09 | offline |
| self test unit tests (`src/worker/self_test.rs`) | prints exactly one JSON line with `selftest`, `whisper`, `ggml`, `isa`, `backend`, `elapsed_ms` and exits 0; an injected unsupported feature set exits 20 with MSG-CPU; a copy of the `for-tests` model with one byte flipped is refused, because its SHA-256 matches neither the `for-tests` pin nor a catalog pin (P-11) | PR-09 | lib |
| supervisor with a fake worker (`src/supervisor.rs`) | framing round trip; the 32 MiB frame cap; a `hello.protocol` other than 1 refused, also for a `BIOROUTER_DICTATION_WORKER` override; each deadline on a test clock (hello 60 s, load 180 s, GPU work max(30 s, 10 × audio), CPU work max(120 s, 20 × audio)); exit on stdin end of file; a crash becomes a typed error and the next request gets a fresh worker; exactly one CPU retry after a GPU fault; `cancel` drops queued segments; first in, first out across streams with round robin by segment; idle unload after 300 s and after `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS`; the last 64 stderr lines kept and never printed raw (DD-6, DD-7) | PR-10 | lib |
| `tests/dictation_worker.rs` (`biorouter-cli`) | with the real worker from `CARGO_BIN_EXE_biorouter`: killed mid request (SIGKILL, or `TerminateProcess` on Windows) gives a typed error and a new worker; a corrupt header is refused before any large allocation. With a fake worker built from the test binary itself, which reports its environment in `hello`: no daemon secret, `GGML_NO_BACKTRACE=1`, no inherited `GGML_*`, `WHISPER_*` or `GGML_VK_*`. A corrupt `config.yaml` and a legacy folder marker under the child's `BIOROUTER_PATH_ROOT` are byte identical after a worker run (the worker reads no configuration and moves no folder) | PR-10 | offline |
| `jfk_with_each_catalog_model` (`#[ignore]`, in `dictation_engine_offline.rs`) | with both catalog models, the transcript of `jfk.wav` equals "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country." after whitespace is normalized; on Apple Silicon it is also run by hand with Metal | PR-12 | Tier 1, by hand |
| daemon binary contract (in `tests/dictation_routes_auth.rs`) | `biorouterd __br-dictation-worker capture` exits 2, because the leaf crate's `run_if_invoked` answers 2 for `capture` in every build; the bytes of the unstripped `CARGO_BIN_EXE_biorouterd` contain neither `ma_device_init` nor `libpulse.so.0` (P-10) | PR-25 | offline |

Capture lives in its own crate, `crates/biorouter-dictation-capture`, which only `biorouter-cli` depends on, and `biorouter`'s `main()` answers the `capture` role before it calls `run_if_invoked` (DD-5). A `capture` feature on the shared leaf crate would not hold P-10: the release builds both binaries in one Cargo invocation (`cargo build --release --bin biorouterd --bin biorouter` [C `scripts/release.sh`, `rust.yml` `cross-build-nightly`]), and the offline step runs `cargo test --workspace` [C], so Cargo compiles the leaf crate once with the union of both binaries' features and links that copy into `biorouterd` too [I]. Three checks hold the separation:

1. `check-dictation-build-prereqs.sh` runs `cargo tree -p biorouter-server -e normal,build` and fails on `biorouter-dictation-capture` (`guards`, from PR-25).
2. `cross-build-nightly` repeats the daemon contract on the binaries its own release command built: `biorouterd __br-dictation-worker capture` exits 2 and `strings biorouterd | grep -c libpulse` is 0 (from PR-25).
3. The package smokes repeat the `strings` check on shipped, stripped binaries (expected 0), because a stripped binary has no symbol table to search.

### Build and packaging

Each build rule of DD-10 has a check that fails when the rule is broken. A rule with no failing check is a rule that will be lost on the next upstream bump.

| Rule (DD-10) | Check that fails when it is broken | Where |
|---|---|---|
| 1. build from a copy in `OUT_DIR` | `scripts/check-vendored-whisper-cpp.sh` run after a build finds the tree byte identical to `source-manifest.json` | PR-09 acceptance; `guards` checks the committed tree |
| 2. `GGML_NATIVE=OFF`, explicit ISA | `scripts/check-dictation-isa.sh` under `qemu-x86_64 -cpu Nehalem`: `biorouter --version` and `biorouterd --version` exit 0, the self test exits 20 with MSG-CPU, `dictation status` reports `unsupported_cpu`; the same container runs the self test natively as a positive control | `cross-build-nightly` |
| 3. static libraries, `GGML_OPENMP=OFF`, no tests, examples, SDL2, curl or ccache | [`check-linux-runtime-deps.sh`](../../scripts/check-linux-runtime-deps.sh) refuses `libgomp.so.1` by name; the Windows import check refuses `libgomp-1.dll` and `vcomp140.dll` | `cross-build-nightly`, `release.sh` |
| 4. static libstdc++ on Linux and mingw | no `GLIBCXX` symbol version and no `libstdc++.so.6` in `DT_NEEDED`; no `libstdc++-6.dll` import | `cross-build-nightly`, `release.sh` |
| 5. mingw links `static:+verbatim=ggml.a` and `advapi32` | a new `cross-build-nightly` step lists `DLL Name` entries of `biorouter.exe` and `biorouterd.exe` with `objdump -p` and fails on `libstdc++-6.dll`, `libgcc_s_seh-1.dll`, `libwinpthread-1.dll`, `libgomp-1.dll` or `vcomp140.dll` [I] | `cross-build-nightly` |
| 6. Metal embedded on arm64, no Metal on x86_64, `GGML_BLAS=OFF` | the macOS self inspection test; `smoke_mac` self test reports `"backend":"metal"` on arm64 and `"backend":"cpu"` on x64 | lib macOS leg, `smoke_mac` |
| 7. `cmake::Config::profile("Release")` | `scripts/check-dictation-build-prereqs.sh` finds `.profile("Release")` in `crates/biorouter-dictation/build.rs`; the Docker workflow's build log shows `CMAKE_BUILD_TYPE=Release` under the Dockerfile's `opt-level = "z"` | `guards`, Docker workflow |
| 8. no bindgen | `check-dictation-build-prereqs.sh` runs `cargo tree -p biorouter-dictation -e build` and fails on `bindgen` | `guards` |
| 9. the vendored tree unchanged after CMake | `build.rs` compares the tree's hashes after CMake and fails the build; the guard of rule 1 catches the rest | every build |
| 10. `cmake` in the Docker builder | `check-dictation-build-prereqs.sh` fails when `crates/biorouter-dictation` exists and the builder stage of the [`Dockerfile`](../../Dockerfile) installs no `cmake` (today it has none [C]) | `guards` |
| 11. `cmake` pinned in hermit | `check-dictation-build-prereqs.sh` fails without `bin/.cmake-<version>.pkg` and `bin/cmake` | `guards` |

**Load time guards land with the engine (DD-12).** PR-09 is the pull request that turns the engine on by default, so it also lands every guard that would catch a binary that cannot start: the Nehalem check, the self test in every package and release smoke (the native Windows publish gate `release-artifact-smoke.yml` included), `DT_NEEDED` within `BASE_LIBS`, no symbol above `GLIBC_MAX` (2.31; the spike measured 2.30 [M]), no `GLIBCXX`, and system DLL imports only on Windows. The minimum OS guard of DD-11 is already merged with PR-02. A reviewer refuses a PR-09 that links the engine without any one of them, because a static initializer that a future compiler vectorizes would break `--version` on older processors before any other gate runs, and the measurement that `--version` survives was taken on a test binary, not on `biorouterd` [M].

Other build and packaging tests:

| Test | Asserts | PR |
|---|---|---|
| `scripts/check-macos-deployment.sh` self test | the minimum OS read with `vtool -show-build` (or from `LC_BUILD_VERSION` or `LC_VERSION_MIN_MACOSX`) must be 12.0; it fails on a fixture built with `MACOSX_DEPLOYMENT_TARGET` cleared. `xcrun dyld_info` rejects binaries built with `strip = true` [M], so the strong import check runs only on unstripped test executables (the self inspection test above) | PR-02 |
| `build.rs` without the variable | building for an Apple target with `MACOSX_DEPLOYMENT_TARGET` unset panics with a sentence naming `.cargo/config.toml` | PR-09 |
| `scripts/check-vendored-whisper-cpp.sh` self tests | an edited, missing, extra or `.gitignore` matched file fails; `pin.json` has `upstream`, `commit` (40 hex digits), `contains: ["60c0be6"]`, `ggml` and `date` | PR-08 |
| `scripts/check-dictation-fixtures.sh` self test | a 1.1 MiB dummy fixture fails; the real four pass at 2,399,463 B | PR-08 |
| runtime deps named refusal | `libstdc++.so.6`, `libgomp.so.1`, `libasound.so.2`, `libpulse.so.0` or `libjack.so.0` in `DT_NEEDED` fails with "link it statically or dlopen it" instead of the existing advice to add a `PACKAGE_MAP` row [C]; the allowlist `BASE_LIBS` is unchanged. A deliberately dynamic libstdc++ build in the PR-09 description shows the message | PR-09 |
| glibc floor | [`check-glibc-floor.sh`](../../scripts/check-glibc-floor.sh) and `assert_glibc_floor` in [`release.sh`](../../scripts/release.sh) read `biorouter`, `biorouterd` and `biorouter-crew`, so they cover the worker with no change [C]; the highest version is `GLIBC_2.30` under `GLIBC_MAX="2.31"` [M] | PR-09 |
| the Crew broker stays free of the engine | `check-dictation-build-prereqs.sh` runs `cargo tree -p biorouter-crew` and fails on `biorouter-dictation` | PR-09 |
| the daemon stays free of capture | `check-dictation-build-prereqs.sh` runs `cargo tree -p biorouter-server -e normal,build` and fails on `biorouter-dictation-capture` | PR-25 |
| binary growth | PR-09's acceptance requires growth within 10 % of +2.17 MB Linux, +3.84 MB Windows, +6.69 MB macOS arm64 and +1.69 MB macOS x86_64 per binary [M]; from PR-11 the size ceiling in the package smokes holds it (see the R6 guards in [Release guards and packaging checks](#release-guards-and-packaging-checks)) | PR-09, PR-11 |
| MSVC leg (M-3) | the Windows `test` leg builds the engine under MSVC (`/MD`), or a `target_env = "msvc"` stub answers `no_engine` and `CLAUDE.md` says which. A follow up study found `cmake` already present on `windows-latest` [M] and estimated the MSVC build at about 1.5 to 3 more minutes on a Windows leg measured at 27.8 minutes [I]. It recommends building the engine over the stub, because a stub leaves the engine's native Windows execution untested on every pull request [I] | PR-09 |
| the `cross_windows` toolchain | `cross_windows` runs in `rust:latest` [C `scripts/cross-env.sh` `WIN_RUST_IMG`], and rustup there already installs and uses the 1.92 toolchain that `rust-toolchain.toml` names: the log of the `rust.yml` `cross-check (x86_64-pc-windows-gnu)` job of 2026-11-02 on `main` shows 1.92.0 auto installed [M]. The spike's rustc 1.98.1 came from a crate built outside the repository, so it shows nothing here. The same log warns that rustup's automatic install is deprecated (`rust-lang/rustup#4836`), so PR-09 adds an explicit `rustup toolchain install` (which installs the toolchain `rust-toolchain.toml` names) to `cross_windows`, and its dispatched `rust.yml` run shows 1.92 in use | PR-09 |
| `NOTICE` pins | a test reads `NOTICE` and asserts the whisper.cpp commit and the miniaudio version equal the code constants; the stale llama.cpp build line equals `LLAMA_SERVER_BUILD` | PR-11 |

### Models and installer

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| catalog invariants (`src/catalog.rs`) | every `sha256` is 64 hex digits; every size is under 2,147,483,648 B; every URL starts with `https://github.com/BaranziniLab/biorouter-models/releases/download/stt-`, ends with the entry's `file_name`, names its `release_tag`, and contains none of `huggingface.co`, `api.github.com` or `/latest/`; every `requires` resolves; exactly one default per hardware class; `accepted_sha256` contains `sha256`; every `RETIRED_STT_MODELS` target exists | PR-11 | lib |
| installer against a loopback HTTP server (`src/models.rs`) | 200; 206 resume with `If-Range`; a wrong `Content-Range`; 416 with a good and a bad partial; a 302 to an expired URL that answers 403, after which the github.com URL is requested again; 429 with `Retry-After`; a sixth redirect refused; a final host outside `release-assets.githubusercontent.com` and `objects.githubusercontent.com` refused (the host list is a parameter only in tests; github.com redirects release downloads only to the first today [M], and the second, GitHub's former asset host, stays allowed as a precaution); no `Authorization` header on any hop; a hash mismatch deletes the file and reports MSG-HASH; a kill and resume across two processes; a second install attaches to the first through the `fs2` lock; `ENOSPC` through an injected space probe gives MSG-DISK; a `.partial` file is never loaded; `HTTPS_PROXY` and `NO_PROXY` are honored (DD-17, DD-21) | PR-11 | lib |
| verification cache | `installed.json` is reused while size and mtime match; touching the file forces a full hash before the next load (P-11) | PR-11 | lib |
| lookup order (`crates/biorouter/src/dictation/paths.rs`) | managed `models_dir`, then `BIOROUTER_DICTATION_MODELS_DIR`, then the per user folder, then the system folder; the first verified copy wins; a copy with a wrong hash in an earlier folder is skipped and left in place; with `model_downloads: false` the managed folder is the only source; `status` reports `location` as `user`, `system`, `managed` or `env` | PR-11 | lib |
| suitability (`src/hardware.rs`) | turbo below 8 GiB and small.en below 4 GiB are `below_minimum`; before `bench`, turbo is `recommended` only on Apple Silicon or with 16 or more cores; a recorded real time factor above 0.5 gives `too_slow`; an unsupported processor gives `unsupported_cpu` for every model; the disk check is `available ≥ remaining + 256 MiB` (DD-22) | PR-11 | lib |
| model commands (`tests/dictation_cli.rs`, `biorouter-cli`) | `install` refused with MSG-INSTALL-SHELL when `BIOROUTER_TERMINAL=1`, even with `--yes`; without a terminal, `install` needs `--yes` (exit 2); `--system` without root or Administrator exits 2; `--from` with a renamed file installs it by hash and reports an unknown file; with `model_downloads: false`, `install` refuses with the policy's sentence (the same text as `status.install.reason`), and a verified file the administrator placed in `models_dir` is found and used; `urls` prints URL, size and SHA-256 per file; `verify` hashes again; `remove` is never refused by policy; removing the last speech model removes the voice detector; `use` refuses an unknown id; a `.en` model refuses a language other than `en` with MSG-LANGUAGE-EN | PR-11 | offline |
| `tests/model_download_live.rs` (`#[ignore]`) | downloads the 885,098 B voice detector from A-1's release end to end; the final host is one of the two allowed; the hash matches | PR-11 | by hand, Tier 1 |
| removal on Windows | delete asks the worker to unload the model first and waits for any load in progress to end (DD-23). The reason is narrower than "an open file": a file opened without `FILE_SHARE_DELETE`, as MSVC's `ifstream` and `fopen` open it, cannot be deleted while it is open, and whisper.cpp closes the model file when loading ends [R], so the conflict exists only during a load, unless a later engine maps the file | PR-11 | lib (Windows leg) |
| `dictation-model-pins.yml` | each catalog `sha256` equals GitHub's reported `digest` for its asset and `gh release verify-asset` passes | PR-11 | nightly |
| no path route (in `tests/dictation_routes.rs`) | no request schema under `/dictation/*` in the generated OpenAPI has a property named `path`, `file`, `dir` or `url`; `POST /dictation/models/rescan` identifies files by SHA-256 and lists the rest in `unrecognized` (DD-24) | PR-15 | offline |
| the R6 packaging guards | the ggml magic scan, `no_model_is_embedded` and the binary size ceiling; see [Release guards and packaging checks](#release-guards-and-packaging-checks) | PR-11 | lib, packaging |

A-1 is accepted by hand before PR-11 merges: each of `stt-silero-vad-v6.2.0-r1`, `stt-whisper-small.en-q8_0-r1` and `stt-whisper-large-v3-turbo-q5_0-r1` is immutable, `gh release verify-asset` passes on each asset, and the reported `.assets[].digest` equals the pin (DD-14). Before PR-12 merges, A-1 also publishes the evaluation asset `eval-primock57-subset-r1` that Tier 1 reads (see [Tier 1](#tier-1-networked-nightly-accuracy)). After that, `dictation-model-pins.yml` repeats the comparison every night.

### Transcription pipeline

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| file decoding (`crates/biorouter-cli/src/dictation/decode.rs`) | generated WAV at 44.1 kHz stereo 16 bit, 48 kHz float and 8 kHz mono, and generated FLAC, each resample to 16 kHz mono with a length within 1 % of the expected; `.opus` and WebM give `unsupported_format` and the `ffmpeg` line; over 200 MB or 2 h gives `too_long`; under 0.5 s gives `too_short` (DD-78) | PR-12 | lib |
| MP3 and M4A decoding (`#[ignore]`, in `dictation_cli.rs`) | the same length check for MP3 and M4A encoded from the generated tone by `ffmpeg`, which the Tier 1 job installs; no encoder for either exists in the offline test process [I] | PR-12 | Tier 1 |
| `transcribe` offline (`tests/dictation_cli.rs`) | `transcribe jfk.wav --format json` with `BIOROUTER_DICTATION_TEST_MODEL` (debug builds only) exits 3 with `{schema: 1, error_code: "no_speech", message}`; generated silence exits 3 before the decoder runs; `transcribe -` reads standard input; every exit code 0 to 5 is reachable in a table test; a successful result serializes to schema 1 with the field names of the HTTP transcript (`text`, `raw_text`, `changes` as `{start, end, from, to, stage, entry_id}`, `audio_s`, `model`, `language`, `bias_scope`, `timings`), checked on the serializer because the `for-tests` model never succeeds | PR-12 | offline |
| whole output filter (`recognize/text.rs`) | every listed phrase and bracketed or parenthesized tag run becomes empty after punctuation is trimmed; a real sentence containing "you" stays (DD-48) | PR-12 | lib |
| echo and collapse (`recognize/echo.rs`) | the whole, fragment and trailing recital rules against the exact prompt sent; fewer than 1.0 output words per second over more than 3 s of detected speech triggers one retry with no prompt and with the detector and the bonus still on; the retry is adopted only if it is not prompt like, else `no_speech`; a trailing recital is stripped and not retried | PR-12, PR-14 | lib |
| decode parameter snapshot | greedy with `best_of = 5`; `en` for `.en` models and the configured language for turbo; `translate = false`; `no_context = true`; `no_timestamps = false` with timestamps stripped from text; `single_segment = false`; temperature 0 with `temperature_inc 0.2`; `entropy_thold 2.4`; `logprob_thold -1.0`; `no_speech_thold 0.6`; `suppress_blank = true`; `suppress_nst = false`; `prompt_tokens` set and `initial_prompt` unset; `flash_attn` on; a request over 25 s of audio is refused by the builder (DD-47) | PR-12 | lib |
| voice detection parameters | threshold 0.50, minimum speech 250 ms, minimum silence 100 ms, `speech_pad_ms = 30`, 0.3 s kept before the first and after the last segment (DD-48) | PR-12 | lib |
| long dictation split (`service.rs` with a fake worker) | no segment request before 8 s are buffered; every queued segment is 25 s or less and is split at the longest pause the fake `vad_segments` reports; the last segment is decoded on stop; every segment is sent with `no_context = true`; a 40 s generated clip yields at least two decodes and none over 25 s (DD-50) | PR-12 | lib |
| symphonia size gate | the stripped Linux `biorouter` with and without symphonia differs by 1.5 MB or less; otherwise AAC and Vorbis are dropped (recorded) | PR-12 | PR-12 acceptance |
| BPE (`engine/bpe.rs`) | `engine/bpe.rs` gives the canonical splits ` EP|AS|1` for ` EPAS1` and ` B|ior|outer` for ` Biorouter`, with golden ids committed as numbers per vocabulary (the English and the multilingual vocabularies assign different ids, so the prompt is encoded per loaded model); `whisper_tokenize` on the `for-tests` vocabularies gives 111, 149 and 62 tokens for the 26, 37 and 14 term lists on the English vocabulary (base.en, shared by small.en) and 119, 160 and 63 on the multilingual one (tiny, shared by turbo) [M]; PR-14 records the canonical counts of `engine/bpe.rs` for the 26 and 37 term lists, which may be up to 2 % higher than those of `whisper_tokenize`; turbo's vocabulary differs from `multilingual.tiktoken` only at id 188 [M] | PR-14 | lib, offline |
| prompt (`recognize/prompt.rs`) | a 600 token dictionary trims to 150 tokens or fewer with the top entry last; entries joined with ", "; no lead in, no tail, no leading space; `collides` entries left out unless in context or pinned; a short list filled with pinned and recent entries, never with a narrow "predicted" list | PR-14 | lib |
| ranking (`recognize/select.rs`) | fixed inputs give the scores of the formula in [dictionary](dictionary.md); ties break by `updated_at` descending, then `id`; `in_context` is a case folded whole word match | PR-14 | lib |
| exact keys (`recognize/key.rs`) | `E-PAS-1`, `EPAS 1` and `E P A S one` all key to `epas1`; a `digits_literal` entry never matches digits that came from number words, so `PhD two` stays | PR-14 | lib |
| correction (`recognize/correct.rs`) | positives corrected: `ventilin`, `klenl`, `FLET3`, `ALAS5AP`, `P-R-O-K-R2`, `IGF-1R`, `E-PAS-1`, `pH D2`. Negatives that must stay: `here's`, `aren't`, `UCSD`, `exon 1`, `N-TRAC`, `clinic`, `continue`, `coding`, `smoke`, `PhD two`, `crisper drawer`, `Oh mop it up`, `The bio router in the closet`, `CRISPR i think`, `encodes`, `cells`, `Boston`; the stop list pairs; the 0.04 abstain margin; wordy aliases only in context; every change listed with its stage and entry id (DD-49) | PR-14 | lib |
| decoder bonus (`engine/booster.rs`) on synthetic logit rows | never boosts an id at or above end of text; identical results from 8 threads; under 100 µs per call; the measured failure cases each a row (a prefix steal of `PhD` into `Ph.D.`, a one character hijack, a first token flip, `PhD two` into `PHD2`); the clamp at 5.0; a panic inside the callback is caught by `catch_unwind` and leaves the logits unchanged (DD-46) | PR-14 | lib |
| bias set (`crates/biorouter/src/dictation/bias.rs`) | the truth table over target tier, entry tier, standing, master switch, `status` and `bias`; `crew` and `external` targets are public unless `include_private_terms` is admitted (the proof and private standing, O-8); `eligible_entries_are_built_only_by_bias_set` walks the repository and finds the `EligibleEntries` constructor called only in `bias.rs` (P-3, P-5) | PR-14 | lib |
| the agent shell rule (DD-77, in `dictation_cli.rs`) | with `BIOROUTER_TERMINAL=1` and a configured private provider, `transcribe --session <private chat id> --format json` reports `bias_scope: "public"`; an `#[ignore]` variant with a real model and reversed speech finds no private term in the text | PR-14 | offline, Tier 1 |
| decoder bonus on the benchmark audio (`#[ignore]`) | 70 % strict or more at 1,000 terms with small.en on the design benchmark's synthetic clips, which are generated by script and never committed | PR-14 | by hand |

### Dictionary store and bias set

Store tests live in `crates/biorouter-dictation/src/store/` (lib); terminal tests in `tests/dictation_cli.rs`; the inspector in `crates/biorouter/src/security/dictionary_store.rs`.

| Test | Asserts | PR |
|---|---|---|
| validation table (`store/validate.rs`) | refused: control characters, CR or LF, `<` or `>`, 65 characters, seven words, a URL, an email, a path, digits only, a long hex string, a secret shaped string; NFC applied; an agent, auto or correction row may not carry a misrecognition that is a common English word | PR-13 |
| rows per tier | a private add of an existing public term creates a second row and leaves the public row byte identical; uniqueness is on (`written` exact NFC, `tier`) | PR-13 |
| fold keys | case fold, spaces, hyphens and dots stripped, Greek letter names to letters; an add whose `fold_key` matches a visible entry is admitted with MSG-SAME-LETTERS; two active entries sharing a `fold_key` disable that key in the matcher | PR-13 |
| tombstones and caps per tier | tombstones per (`fold_key`, `tier`); caps of 5,000 active, 200 pending suggestions, 200 unreviewed agent rows and 5,000 tombstones count only the caller's tier; above a cap `add` and `on_request` fall back to `suggested` (DD-41) | PR-13 |
| concurrency | a suggestion racing a delete loses nothing; a usage bump leaves `rev` unchanged; a term added by one process is listed by a second process at once (cache by mtime and length) | PR-13 |
| corrupt store | a file that fails to parse refuses every write with MSG-DICTIONARY-FILE, lists nothing, and stays byte identical | PR-13 |
| `the_tier_proof_is_constructed_in_exactly_two_files` | `UserDictionaryTierChange::from_user_action()` appears only in `crates/biorouter-cli/src/commands/dictionary.rs` and `crates/biorouter-server/src/routes/dictionary.rs`. PR-13 lands it with the CLI file and refuses any third file; PR-16 makes the count exactly two (P-13) | PR-13, PR-16 |
| CLI dictionary commands | every write except `import` is refused without a terminal (exit 2); every write, `import` included, is refused when `BIOROUTER_TERMINAL=1`; a piped `import -` merges, because the sync path of DD-43 (`biorouter dictionary export - \| ssh <host> biorouter dictionary import -`) runs `import` with a pipe on stdin, where `prompt_can_run()` is false [C `needs_terminal.rs`]; `list` piped withholds private rows and prints no count of them; `publicize` and `clear` need the typed phrase; `export --include-private` only at an interactive terminal with private capability; import plans `new`, `merged`, `duplicate` and `invalid(reason)` for lines, CSV, TSV and the export JSON, and never deletes; export carries no ids, session ids or usage | PR-13 |
| store inspector | `cat <config>/dictionary/dictionary.json` through `developer__shell` and `text_editor view` of the file are refused for both tiers (P-7, P-14) | PR-13 |
| path agreement | a row for `<config>/dictionary/` in [`path_resolver_agreement.rs`](../../crates/biorouter/tests/path_resolver_agreement.rs), including `$BIOROUTER_PATH_ROOT/config/dictionary/` | PR-13 |

The bias set tests are in the transcription table above, because they run with the recognition code.

### Routes

| Binary (`biorouter-server`) | Asserts | PR |
|---|---|---|
| `tests/no_cloud_audio_routes.rs` | `POST /audio/transcribe`, `POST /audio/transcribe/elevenlabs` and `GET /audio/config` answer 404 through `routes::configure` and `check_token`; a source scan under `crates/` and `ui/desktop/src` finds no `api.openai.com/v1/audio` and no `elevenlabs.io` (DD-89, P-1). At `main` the inherited cloud dictation is only these cloud routes (OpenAI's transcription API and ElevenLabs) and their renderer code; nothing spawns a local `whisper-server` [C] | PR-01 |
| `tests/dictation_routes.rs` | 201, 409 `noModel`, 410, 413 (a chunk over 524,288 B, a JSON body over 8 KiB, a `context` over 2 KiB), 429 `tooManyStreams` with `Retry-After` for a ninth stream and for a 33rd queued segment, 202 after 3 s then `GET`; a resent `seq` is idempotent; `409 outOfOrder` carries `expected_seq`; a stream expires after 60 s idle on a test clock; a 5 s stream finishes within two segment times while a 10 minute stream runs; a PUT that would pass 600 s of audio is refused and `finish` still answers [I]; `MemAvailable` and cgroup `memory.max` below the model's figure give MSG-SERVER-MEMORY through an injected probe; no new file under the sandbox root after a stream, and a tracing capture holds no audio derived content, transcript, prompt or `context` (P-1, P-2, DD-93); a refused `session_id` gives `bias_scope: "public"` and a byte identical response; a `new_chat` stream whose `new_chat_provider` names a private model is judged by `BIOROUTER_PROVIDER` when the caller sends no proof, and by that model with the proof (DD-30, P-4); for `new_chat`, an `ollama` cloud tag, a remote `OLLAMA_HOST` and a private worker under a public lead each give `bias_scope: "public"`, because the decision takes the tier the bind would produce, not a declared tier; a chat bound to a private provider whose row is still public gives private scope through `chat_reach`; `include_private_terms` on a `crew` or `external` stream admits private entries only with the proof and private standing; a worker that exits mid stream answers `500 engineFailed` from `finish` and from a poll; the echo oracle: a secret only caller sending near silence with a private chat's id gets public scope and a byte identical response (P-6); no response field counts eligible or withheld entries | PR-15 |
| `tests/dictation_routes_auth.rs` | every `/dictation/*` route answers 401 without the secret; the daemon binary contract of P-10 (added by PR-25) | PR-15, PR-25 |
| `tests/dictation_keyed.rs` | install, select and delete without the proof are refused with MSG-INSTALL-AGENT; streams are admitted on the secret alone (O-17 recommended; if the owner chooses the alternative, this binary pins the refusal instead); private bias needs the proof; `GET /dictation/status` gives an unproven caller `install: {allowed: false, reason}` (P-7) | PR-15 |
| `tests/dictation_no_user_key.rs` | install admitted on the secret; refused with MSG-INSTALL-LOCKED under `BIOROUTER_DICTATION_BROWSER_INSTALL=false`; a private host's `X-Caller-Provider` gives private standing; a served page request whose only private input is the operator's tier (`served_operator`) gets public scope (DD-30) | PR-15 |
| `tests/dictation_policy_routes.rs` | `enabled: false` refuses streams, install and select with MSG-POLICY; `capture: false` refuses stream creation with MSG-POLICY-CAPTURE; `model_downloads: false` refuses install while models in `models_dir` load; `allowed_models` refuses others; delete is never refused; an unknown key turns dictation off with one warning naming it; every `policy` entry in status is `{allowed, reason}` (P-15) | PR-15 |
| `tests/dictionary_routes.rs` | listing, `ETag` over the caller's filtered view, `If-Match` with a stale `rev` refused [I: 412]; add, edit, delete, bulk delete over listed ids, keep, accept, dismiss, import dry run and merge, export; observations idempotent on `(record_id, from, to)` with PII and secret pairs dropped; a correction in a chat bound to a private provider whose row is still public lands in a private row (`chat_reach`); managed `dictionary: false` refuses writes and observations with MSG-POLICY while export and delete stay allowed; `POST /dictionary/clear` answers 404 | PR-16 |
| `tests/dictionary_routes_auth.rs` | every route needs the secret; on a keyed daemon every write needs the proof; `POST /dictionary/entries/{id}/tier` checks the proof before the id is read | PR-16 |
| `tests/dictionary_no_user_key.rs` | add, accept, edit and delete admitted for listable rows, stamped with the caller's standing; publicize and privatize refused with MSG-DICTIONARY-HOST-TIER; raising `agent_writes` and turning `learn_from_chats` on refused with MSG-DICTIONARY-HOST-SETTINGS, turning it off admitted; a `private: true` add or import, or an imported row marked private, from a caller without private standing refused with MSG-DICTIONARY-HOST-PRIVATE before any row is read; `GET /dictionary/settings` reports `authority.add_private` and `authority.raise_settings`; a private host's `X-Caller-Provider` lists private rows | PR-16 |
| `tests/dictionary_oracles.rs` | one byte identical test per rule of DD-40: public add consults only public rows and tombstones; an unlistable id answers exactly as a missing one (MSG-NOT-FOUND); dry runs and the "same letters" warning count listable rows only; no store wide revision; usage and `suggested_in` need private standing; dictation response shape identical across scopes; refusals name no term; caps count only the caller's tier; bonus tables rebuilt for both scopes at each revision; no eligible or withheld count anywhere; a public scope prompt and fuzzy set are identical before and after private scope dictations and private corrections (rule 11, the two usage maps); a keyless `private: true` add from a caller without private standing gives the same 403 for an existing private term and an absent one; such a caller's observation that names no chat is a silent 204 and leaves the store byte identical (P-6) | PR-16 |

Also in PR-15 and PR-16: `scripts/check-openapi-schema.sh` is clean after `just generate-openapi && cd ui/desktop && npm run generate-api`; the TypeScript client compiles; the OpenAPI diff shows no dictation field on `Message` (P-2). The census in [`privacy_guard_wiring.rs`](../../crates/biorouter/tests/privacy_guard_wiring.rs) gains rows for `lists_dictionary_entry`, `dictation_standing`, `authorizes_dictionary_write`, `dictionary_write_stamp` and `gate_dictionary_entry` (`Status::Wired`, `defined_in: SESSION_REACH`), rows written by hand for `bias_set` (`crates/biorouter/src/dictation/bias.rs`) and `chat_reach` (`crates/biorouter/src/dictation/target.rs`), the `visible_to` site in `bias.rs`, and a site for `target_tier` in `routes/dictation.rs`, which moves that row from `Status::WiredThrough("session_reach")` to `Status::Wired`. `private_standing_is_built_in_two_files` pins `Standing::from_checked` to `HttpCaller::dictation_standing` and `crates/biorouter-cli/src/dictation/consent.rs`. No pull request adds a `floor`, `raise_privacy`, `.call_tool(` or `CallCapability::sample(` site, and the census's own counts fail one that does.

### The capability and the extractor

| Test | Asserts | PR |
|---|---|---|
| tool shapes | the capability's tool list is exactly `dictionary__add_terms`, `dictionary__list_terms` and `dictionary__remove_added_terms`; an unknown `tier` field is rejected; an 11th term in one call is refused (P-7) | PR-23 |
| stamps and listing | the stamp follows `McpMeta.capability`; `list_terms` omits rows outside the caller's tier and reports no count; an `execute_code` script call of `dictionary__add_terms` or `dictionary__remove_added_terms` is refused, also when the script ran through `/agent/call_tool` (DD-42) | PR-23 |
| modes | `suggest` never produces an active row; `on_request` activates, with `requested_by_user` true, only a term found by fold key in the latest person written user message or in the assistant message that person message directly answers, never from `AgentInjection` or `SpawnContext` provenance: an assistant offer naming EPAS1, then the person's "yes", then `add_terms` with `requested_by_user` lands EPAS1 active and unreviewed with **Keep** and **Undo**, while a term found only earlier in the conversation lands suggested; `add` lands active and unreviewed until the caps; the 31st term per session and the 201st unreviewed row land `suggested`; ACP stdio always suggests | PR-23 |
| what an agent row may carry | no `spoken_forms` or `misrecognitions`, `match: exact`; `remove_added_terms` cannot touch a person's row and writes no tombstone | PR-23 |
| where the capability is absent | not offered in Crew scoped, app, ACP `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; an app manifest naming `dictionary` gets `UnarmedExtension::Platform`; `/agent/call_tool` with `dictionary__add_terms` is refused by `call_tool_boundary_refusal`; the same tool from `execute_code` is refused by `uninspected_boundary_refusal`; a resumed roster under managed `dictionary: false` does not load it; `an_agent_cannot_enable_the_dictionary_capability_in_any_chat` drives `workspace_set_tools`, `workspace_open`, `WorkspaceServices::start_session` and `manage_extensions` and finds each refused with "The Dictionary capability is turned on by you, with Suggest dictionary words from my chats on the Dictionary page." (DD-51) (P-9, P-15) | PR-23 |
| instructions | a snapshot of the capability's instructions contains no term text (P-14) | PR-23 |
| enablement | off in a fresh configuration; after the first verified install `dictation_dictionary_activation_version: 1` is written whatever the box says, and the capability is turned on only when the box was ticked; once the marker exists, no later install offers the box or turns the capability on; a later "off" sticks; `--yes` without `--suggest-from-chats` leaves it off; the marker is hidden in `ConfigSettings.tsx`; `POST /dictionary/activation` needs the proof on a keyed daemon, and on a keyless one admits `false` and refuses `true` with MSG-DICTIONARY-HOST-SETTINGS; `biorouter dictionary settings --suggest-from-chats on` asks before it turns the capability on and goes through `activation.rs` | PR-23 |
| hand synced lists | `PLATFORM_EXTENSIONS.len()` changes from 7 to 8 in both count tests [C `agents/extension.rs:943`, `:952`]; `capabilities.test.ts` and `toolGlyph.test.ts` pass with `dictionary` added | PR-23 |
| extractor through `Agent::reply` | `AgentInjection` provenance and SubAgent, Scheduled, Terminal, Crew scoped, app, ACP stdio and `--ws` sessions produce nothing; slash commands, tool output and assistant text produce nothing; a private chat produces a private suggestion; a token `PiiDetector::scan` flags is skipped; `seen` counts once per session and a candidate is listed only at `seen ≥ 2`; at most 3 per message; the `/interrupt` admission path for a steer that is not a duplicate; no provider call (a provider double counts zero); a tracing capture holds no message text | PR-24 |
| extractor budget | p99 under 50 ms with a 5,000 entry store and two writers; contention beyond the 50 ms `try_lock` budget drops the candidates | PR-24 |

**The gate of DD-57 (M-21).** `learn_from_chats` ships on only if the extractor reaches precision 0.8 or more at `seen ≥ 2` on the person written messages of 200 of the owner's sessions. Those sessions are the owner's own chats and may hold patient data, so the run needs the owner's consent and someone's labeling time: that is part of owner decision O-23, beside the lab recording set (recommended: yes, run by the owner on the owner's machine, leaving out private tier sessions). The run is local and nothing leaves the machine:

1. The owner copies `sessions.db` to a private folder with Biorouter closed.
2. An `#[ignore]` test, `crates/biorouter/tests/dictation_extractor_precision.rs`, reads that copy (path in the test only variable `BIOROUTER_EXTRACTOR_EVAL_DB`), takes the 200 most recent sessions whose `privacy_tier` is public, leaves out any other session the owner names (for example one with patient data), runs `extractor::candidates` over person written user messages only, applies `seen ≥ 2` across sessions, and writes one CSV row per listed candidate to a temporary folder outside the repository. A row holds the candidate token and its count, never the message around it.
3. The owner, or a person the owner names, labels each row "would add" or "would not".
4. Precision is the share labelled "would add". The PR-24 description records the date, the count of candidates and the precision, never a term.
5. At 0.8 or more the default is on; otherwise PR-24 ships it off (`LEARN_FROM_CHATS_DEFAULT = false`), the Dictionary page shows **Also find terms in my messages automatically** off, and the reason is recorded in [dictionary](dictionary.md). If the owner declines this part of O-23, the run does not happen and the default ships off with that reason.

On public text, a close variant of these candidate rules reached 74 % to 75 % precision at `seen ≥ 2`, and requiring a second sighting did not raise precision (it cut volume about six times) [M]. So the default most likely ships off unless the owner's own sessions do better [I].

### Desktop capture and composer

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| `permissionPolicy.test.ts` | `media` granted only for the main frame and audio only; `about:srcdoc`, `isMainFrame: false` and `['audio','video']` refused | PR-04 | vitest |
| `styles/forgeMacUsage.test.ts` | `extendInfo.NSMicrophoneUsageDescription` equals "Biorouter uses the microphone only while you dictate, and does not save the recording."; `osxSign.optionsForFile` adds `com.apple.security.device.audio-input` for the app, `Biorouter Helper.app`, `Resources/bin/biorouter` and `Resources/bin/biorouterd` | PR-04 | vitest |
| `UserMessage.tsx` log grep test | no `logInfo` template in `UserMessage.tsx` carries message text (DD-69, P-2) | PR-04 | vitest |
| `components/dictation/transport.test.ts` (fake fetch and timers) | exactly one PUT in flight; the same `seq` resent after the 10 s timeout; a 413 and a network error on a chunk over 64 KiB halve it, with a floor of 64 KiB; `409 outOfOrder` resends from `expected_seq`; 202 polling every 500 ms without a held request; MSG-WAIT after 1.5 s without an answer; one replay after a 410, then MSG-LOST; MSG-CAP-WARN at 570 s and MSG-CAP-STOPPED at 600 s; a `500 engineFailed` gives MSG-ENGINE-FAILED with **Retry**, which replays the buffered chunks into a new stream; no long lived request is ever opened | PR-17 | vitest |
| `dictationStore.test.ts` | no request on `ChatInput` mount, so the 23 test files that render the real composer under an offline fetch guard pass unchanged; the first request on the first focus of a composer textarea, or on `pointerenter`, focus or click of a microphone; a click or ⌘⇧D that arrives before the store has loaded still reaches recording; polling every 1.5 s only while an install runs; `!window.isSecureContext` or no `getUserMedia` gives `unavailable` before any request | PR-17 | vitest |
| `micErrors.test.ts` | each row of the error map: `NotAllowedError` with OS `denied`, `restricted` and `granted`; browser denial; `NotFoundError`; `NotReadableError` and `AbortError`; insecure context | PR-17 | vitest |
| `utils/dictatedText.test.ts` | `spliceDictation` spacing, the stripped leading `/`, no case change; `locateEdit` maps an anchor through edits; the focus guard falls back to the state path; `insertIntoComposerDraft` when the composer unmounted | PR-17 | vitest |
| source guards | the worklet import carries `?url&no-inline`; no `blob:` or `data:` worklet; no `use-fake-ui-for-media-stream` anywhere under `ui/desktop/` | PR-17 | vitest |
| `dictation-capture-electron` | see [CI jobs](#ci-jobs): the worklet loads from `app.asar` under the real content security policy; RMS and pitch of a generated tone; sample count; permission refusals; tracks ended | PR-17 | Electron job |
| `DictationButton.test.tsx` | every phase's look, name and tooltip; `aria-pressed`; `aria-keyshortcuts`; a tap against a 500 ms hold; Escape `defaultPrevented`; mousedown keeps focus in the textarea | PR-19 | vitest |
| `DictationButton.browser.test.tsx` | pill geometry at 760, 440 and 360 px; the timer dropped under a 440 px pane; static level bars under `prefers-reduced-motion` | PR-19 | vitest (browser) |
| `ChatInput.structure.test.tsx`, `composerGeometry.browser.test.ts`, `composerCard.browser.test.tsx` | the controls row reads model, then microphone, then **Send**; the footer inset still derives from **Send**'s edge | PR-19 | vitest |
| `ChatInput.dictation.test.tsx` | all three submit paths refused while arming, recording or transcribing; Enter while recording stops and inserts and never sends; Stop for a running turn stays available; MSG-PRIVATE-DRAFT before a send to a public model and MSG-PRIVATE-DRAFT-SWITCH before a switch to one (P-8) | PR-19 | vitest |
| one undo step | a dev app Playwright run (`BIOROUTER_NO_HMR=1`, fake device) shows one ⌘Z removing exactly the dictated words in the real `ChatInput`, and nothing sent without Enter. The launch steps are in [launching the dev GUI](../desktop-ui/launching-the-dev-gui.md) | PR-19 | by hand, recorded |
| correction learning (`recognize/learn.rs`) | golden pairs: `Epas one` to `EPAS1` accepted; `gonna` to `going to`, `the` to `a`, a URL, an AWS key and an MRN rejected; observations idempotent; a cancelled send posts nothing; nothing persisted or logged | PR-22 | lib |
| stale pairs (browser test) | a later programmatic change to the draft posts no stale pair | PR-22 | vitest (browser) |

### Sidebar and Settings

| Test | Asserts | PR |
|---|---|---|
| `ModelsSection.test.tsx` | the Models tab has four sections, the speech section between the local model inventory and the danger zone | PR-18 |
| `settingsSections.test.ts` | the deep link set gains `dictation` and nothing else changes | PR-18 |
| `speechModelStore.test.ts` | install progress survives navigation; polling only while an install runs; install, select and delete send `userActionHeaders()`; a refused install shows `status.install.reason` in place of the button; MSG-TURBO-CPU names the measured `bench` time (P-8) | PR-18 |
| `modelsGeometry.browser.test.tsx` | the section's rows hold the shared geometry | PR-18 |
| a vitest per hidden control | before PR-21, the Settings section, the microphone and the sidebar row are absent on `isBrowserSurface()` | PR-18, PR-19, PR-20 |
| `parseTermList.test.ts` | lines, `EPAS1 = epas one, e pass one`, `#` comments, CSV and TSV parse into the daemon's import shape | PR-20 |
| `DictionaryView.test.tsx` | the groups **Suggested**, **Added by the agent**, **Added by you**, **From your chats**, **Imported**; a query collapses them into **Matches**; review rows; MSG-PUBLICIZE-WARN before **Make public**; MSG-EXPORT-PRIVATE before an export with private rows (P-8) | PR-20 |
| `useDictionary.test.ts` | `userActionHeaders()` on every call; stale responses dropped | PR-20 |
| `dictionaryRows.browser.test.tsx` | no row moves under a hovering pointer; incoming rows wait until `pointerleave` | PR-20 |
| sidebar reveal | the Components disclosure opens once after the first verified install (`biorouter:sidebar-components-expanded` written once, marker `biorouter:dictionary-revealed`); a later collapse sticks | PR-20 |
| guard files | `node landing/scripts/check-consistency.mjs`, `tests/e2e/helpers/sidebar.ts` `COMPONENT_LABELS`, `styles/measures.test.ts`, `settings/settingsVocabulary.test.ts`, `App.routing.test.tsx`, `Layout/AppLayout.test.tsx` and `BioRouterSidebar/AppSidebar.test.tsx` updated for the seventh destination | PR-20 |
| context menu labels | a main process test: the spelling item reads **Add to spelling dictionary**; **Add to Biorouter dictionary…** appears for a misspelled word or a selection of at most 64 characters and sends `dictionary:add-term` with `{text, x, y}`; a renderer test: a selection in a private chat's pane, while a public chat is current, opens `TermDialog` with **Only in private chats** ticked, and so does one in a transcript it cannot resolve, such as a shared one | PR-20 |

### CLI capture and front ends

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| `dictation status` and `doctor` | `status --format json` validates against schema 1; the **Dictation** section of `doctor` sits between "Local models" and "Shell sandbox"; from PR-10, `capture` is `{available: false, reason: "not_compiled"}`, because capture lands only with PR-25; from PR-25, in the package smoke containers `capture.available` is false with `reason: "no_audio_system"` and stderr has no `ALSA lib` line | PR-10, PR-25 | lib, packaging |
| capture child frames | `ready`, `level` every 100 ms, `error`, `stopped`; audio frames `[u32 len][f32 LE]`; stdin end of file exits within 2 s; killing the child releases the device; the Null backend is refused; ALSA's stderr does not reach the terminal | PR-25 | lib |
| controller with a fake source | exact zeros for 1.5 s give the denial path (MSG-TCC-ZERO or MSG-WIN-ZERO); under 0.5 s gives MSG-TOO-SHORT; below the noise floor for 3 s shows MSG-NO-AUDIO-YET and keeps recording; the 600 s cap with MSG-CAP-WARN at 570 s and MSG-CAP-STOPPED at 600 s; three failures within 10 s pause dictation with MSG-PAUSED; a failed transcription gives MSG-ENGINE-FAILED-CLI; `/dictate retry` keeps the last recording for at most 5 minutes | PR-25 | lib |
| environment detection table | the order of `environment.rs` ([surfaces](surfaces.md)): not compiled, policy, `BIOROUTER_TERMINAL=1`, no terminal, macOS over SSH, CPU gate, then the capture probe (backends and devices listed, none opened) with its wording for SSH (variables, or an `sshd` or `sshd-session` ancestor, with stale tmux variables; MSG-SSH-SERVICE when a `biorouter serve` runs there, else MSG-SSH-ROUTES), a container (`/.dockerenv`, `/run/.containerenv`, a Kubernetes pod through `KUBERNETES_SERVICE_HOST` or `/proc/1/cgroup`), WSL1, WSL2 without WSLg (MSG-WSL2-NOWSLG), WSL2 without `libpulse.so.0`, and other Linux (MSG-LINUX-NOAUDIO then MSG-LINUX-ROUTES), then no model, then the device, then zero samples; no model plus SSH gives MSG-SSH-ROUTES, exit 2 and no install prompt; each case picks its message and exit code | PR-25 | lib |
| message snapshots | every entry of `crates/biorouter-cli/src/dictation/messages.rs` | PR-25 | lib |
| exit codes and JSON | `dictate` and `transcribe` exit 0 to 5 as specified; error JSON `{schema: 1, error_code, message}` | PR-25 | offline |
| review | `"$(biorouter dictate)"` and a pipe are reviewed on the terminal before anything leaves (Enter uses, `e` edits, Escape discards) | PR-25 | offline (pseudo terminal) |
| `capture_consent_is_minted_in_one_file` | `CaptureConsent` is constructed only in `crates/biorouter-cli/src/dictation/consent.rs` (P-7) | PR-25 | lib |
| clipboard helper | with an injectable backend: macOS set and exit; Windows through arboard's `SetExtWindows::exclude_from_cloud()`, which adds the `CanUploadToCloudClipboard` format (arboard 3.6.1 is already a `biorouter-cli` dependency [C `Cargo.toml`]); Linux wait up to 60 s with MSG-CLIPBOARD-WAIT after 300 ms; WSL through `powershell.exe Set-Clipboard`; no display prints and exits 5 with MSG-NO-CLIPBOARD; `--no-history` adds `exclude_from_history()` | PR-25 | lib |
| link checks | on Linux `readelf -d biorouter` shows no `libasound` or `libpulse`; the packaged CLI's entitlements list `audio-input` | PR-25 | packaging |
| `App` unit tests (TUI) | the insert anchor, spacing and edits made during transcription; Ctrl+C while recording during a stream yields no `StreamAction::Cancel`; the newline key collision disables dictation's key with one startup note, MSG-KEY-CLASH | PR-26 | lib |
| `draw_input` snapshots | `◌ Starting microphone…`, `● REC 0:07 ▂▅▇▅▂  Enter insert · Esc cancel`, `◌ Transcribing 0:07…`, the `^T dictate` hint only when compiled in, allowed and installed; `NO_COLOR` keeps `REC` and the timer | PR-26 | lib |
| pseudo terminal test | a TUI session with a fake capture source: Ctrl+T, Escape, Ctrl+C; one insert, an edit and Enter yield one correction pair; the classic REPL's Ctrl+T dictates instead of transposing, and there too one insert, an edit and Enter yield one correction pair (the `readline_with_initial` seed diffed against the submitted line) | PR-26 | offline |
| `session attach` | `/dictate` is never posted as a steer; `Send? [y/N]` defaults to No | PR-26 | lib |

### `serve`

| Test | Asserts | PR | Runs in |
|---|---|---|---|
| header unit tests | a new constant that PR-03 adds, `APP_PERMISSIONS_POLICY = "microphone=(), camera=(), display-capture=()"`, on the responses of `app_page_response`, `with_app_file_csp`, `figure_frame` and `launch_bounce` in `routes/apps.rs`; `Permissions-Policy: microphone=(self)` on the shell that `index` in `routes/web_ui.rs` serves. Nothing at `main` sends a `Permissions-Policy` header [C] | PR-03 | lib |
| app page cannot use a shell grant | Playwright Chromium grants the microphone to the shell's origin, opens an app page on the same origin and gets `NotAllowedError` from `getUserMedia`; Firefox and WebKit are recorded as the documented residual | PR-03 | `serve` |
| browser capture | Playwright Chromium (the 16 kHz path) and Firefox (the native rate fallback) against `biorouter serve` on loopback with the debug test model: the composer shows MSG-NOTHING after a fake microphone dictation, and the daemon's log reports received audio bytes equal to 32,000 B per second of recorded time, within one chunk. Before Firefox 148, `new AudioContext({sampleRate: 16000})` succeeds and `createMediaStreamSource` throws `NotSupportedError` for a microphone stream at another rate [M on Firefox 146]; Firefox 148 allows that connection (bug 1674892) [R]. If the Playwright Firefox build is 148 or later, the test forces the fallback by stubbing `createMediaStreamSource` to throw `NotSupportedError` once, the throw site of the real failure, so the path stays covered [I] | PR-21 | `serve` |
| path prefix and 413 | an `nginx:stable` container serves the shell under `/user/a/biorouter/`; the worklet resolves; a raw PUT of 1 MiB plus 2 B gets nginx's readable 413; with `client_max_body_size 128k` in the container, the client halves its 512 KiB chunks and continues; with the setting `browser-access.md` requires (`client_body_buffer_size 512k`), nginx's error log holds no "buffered to a temporary file" line after a dictation (P-1; the stock configuration wrote every chunk above 16 KiB to disk [M]) | PR-21 | `serve` |
| build output | `npm run build:web` emits the worklet as an `assets/*.js` file that the entry chunk references; removing `?no-inline` turns the job red | PR-21 | `serve` |
| environment checks (WebKit) | `isSecureContext` is true at `127.0.0.1`; a plain HTTP origin that is not loopback has no `navigator.mediaDevices` | PR-21 | `serve` |
| banner snapshots (`commands::serve`) | "Dictation: ready (<model>)", the no model line and the unsupported processor line; four binds: loopback (no extra line), `0.0.0.0` (MSG-SERVE-TUNNEL in place of the plain HTTP LAN address), `0.0.0.0` inside a container (MSG-SERVE-CONTAINER), and `--host 0.0.0.0 --public-url https://hub.example/user/a/biorouter/` (no extra line); `--no-dictation-install` sets `BIOROUTER_DICTATION_BROWSER_INSTALL=false` for the daemon it starts | PR-21 | lib |
| served page gates | **Show in Finder** hidden; publicize and privatize disabled before the click with MSG-DICTIONARY-HOST-TIER; raising `agent_writes` and turning chat learning or suggestions on disabled with MSG-DICTIONARY-HOST-SETTINGS; **Only in private chats** disabled without private standing with MSG-DICTIONARY-HOST-PRIVATE; MSG-SHARED-ORIGIN shown once per origin when the page lives under a path prefix or the browser does not enforce `Permissions-Policy` | PR-21 | vitest |

### Jupyter

The new workflow `jupyter-ai.yml` runs these against `integrations/jupyter-ai/`.

| Test | Asserts | PR |
|---|---|---|
| `pytest-jupyter` handler tests, with a fake `biorouter` | handlers require authentication and XSRF; chunks over 512 KiB, more than 8 MiB in total or a fifth open upload are refused; a 5 minute idle upload expires; one job at a time; a 180 s timeout kills the process group; `DELETE` kills it too; the WAV reaches the child on stdin and no file appears under `TMPDIR` (P-1); nothing about audio or text is logged; the child is `biorouter transcribe - --format json` and is passed no session (P-5); under managed `capture: false` the upload and finish routes answer 403 with MSG-POLICY-CAPTURE (MSG-POLICY under `enabled: false`), although `transcribe` itself is allowed; a timeout, a nonzero exit and a proxy's 413 each give MSG-JUPYTER-FAILED | PR-32 |
| import without `biorouter` | importing `jupyter_ai_biorouter` with no `biorouter` on `PATH` succeeds (the resolver moved out of import, [C `persona.py`]); the status handler then answers with MSG-JUPYTER-NO-BINARY | PR-32 |
| Jest | the caret insert (splice `value`, set `cursorIndex`, `focus()`; never `send()`, never an `@Biorouter` mention); the WAV encoder at 16 kHz mono 16 bit; the button disabled before any click with MSG-JUPYTER-INSECURE on an insecure page and with MSG-JUPYTER-NO-MODEL when no model is installed | PR-32 |
| Galata smoke | JupyterLab 4.5.9 with Jupyter AI 3.0.1: the microphone sits at position 90 left of **Send** in every tracked chat widget, including one opened later; MSG-JUPYTER-RECORDING and MSG-JUPYTER-TRANSCRIBING appear; the text lands at the caret and nothing is sent | PR-32 |

### ACP

| Test | Asserts | PR |
|---|---|---|
| audio block | an ACP prompt with `ContentBlock::Audio` yields MSG-ACP-NOTE in the turn and no audio reaches the agent; today the arm drops it silently [C `server.rs:428`] | PR-30 |
| capabilities | `on_initialize` still advertises `audio(false)` [C `server.rs:755`] | PR-30 |
| `/dictate` | at session start the server sends `AvailableCommandsUpdate` with a `/dictate` command; invoking it answers MSG-ACP-DICTATE and a provider double counts zero calls | PR-30 |
| `--ws` | a `--ws` agent loads no `dictionary`, and `--with-builtin dictionary` is refused there | PR-23, PR-30 |
| stdio | ACP stdio offers the capability in suggest form only and runs no extractor; the tool answer appends MSG-TOOL-REVIEW-HINT | PR-23, PR-24 |

`cargo test -p biorouter-acp` is the acceptance for PR-30.

### Crew

| Test | Asserts | PR |
|---|---|---|
| end of dictation | never calls `submitNow` or `sendNow`; a dictated `@agent …` starts nothing without Enter; the draft's idempotency key is unchanged | PR-27 |
| caret | `useRestoredDraftCaret` leaves the caret after an `execCommand` insert (browser test) | PR-27 |
| bias | requests carry `target_kind: "crew"`; the daemon gives public scope unless the recording carries `include_private_terms`, sent only after the person confirms MSG-PRIVATE-TERMS and honoured only with the proof and private standing; the microphone's context menu offers **Include my private terms** only where the caller holds both; the choice lasts one recording (P-5, O-8) | PR-27 |
| manual check | `node scripts/check-crew-manual.mjs` passes with a line in `docs/crew/` for every new quoted string, MSG-CREW-TIP included | PR-27 |

### Clipboard

| Test | Asserts | PR |
|---|---|---|
| `dictationAnywhere.test.ts` state machine | shortcut taken (MSG-SHORTCUT-TAKEN), policy off, no model, an echo only result (MSG-CLIPBOARD-ECHO), nothing heard (MSG-CLIPBOARD-NOTHING, clipboard unchanged), a read back mismatch opens the review card with **Copy** | PR-28 |
| notifications | carry counts only, never words (P-2) | PR-28 |
| target | every request carries `target_kind: "external"`, so the daemon gives public scope unless the person chose **Include my private terms** (the tray item below **Dictate to clipboard**, or the preview button's context menu) and confirmed MSG-PRIVATE-TERMS for that recording; the global shortcut always records with public terms (P-5, O-8) | PR-28 |
| who may start capture | no daemon route and no tool starts capture; only the tray, the person's shortcut or the preview button does (P-7) | PR-28 |
| preview insert | **Dictate into this app** is absent when nothing in the preview is focused, and is removed from PR-28 entirely if M-15 fails | PR-28 |
| Copilot | `press_key` refuses the person's configured global shortcut on any target, read from `<config>/dictation/anywhere.json` on each call (DD-88); only Electron main writes that file, and `/config/upsert` cannot change it because it is outside `config.yaml` | PR-28 |
| CLI | the clipboard helper rows under [CLI capture and front ends](#cli-capture-and-front-ends) | PR-25 |

### Docker

The Docker workflow (nightly and dispatch only, O-24) proves PR-31, and the `cmake` prerequisite guard proves PR-09's Dockerfile change on every pull request.

| Check | Asserts | PR |
|---|---|---|
| image build | `docker build` succeeds on amd64 and arm64; the build log shows `CMAKE_BUILD_TYPE=Release` | PR-31 |
| self test | inside the container, `biorouter __br-dictation-worker self-test` exits 0 with the fixtures mounted read only | PR-31 |
| no model (R6) | `biorouter dictation status --format json` reports no installed model; no file in the image starts with the ggml magic | PR-31 |
| model in a volume | `docker run … <image> dictation install --yes whisper-small.en-q8_0` writes into the data volume at `/home/biorouter/.local/share/biorouter` (cached by SHA-256 between runs); the image itself stays unchanged | PR-31 |
| `serve` | `serve --host 0.0.0.0` published on `127.0.0.1:8765` serves the shell; a Playwright Chromium page there reports `isSecureContext`, and a fake microphone playing `jfk.wav` yields the JFK sentence in the composer (M-16). The image is a release build, so the debug only `for-tests` switch is not available and the real Light model is used | PR-31 |
| `transcribe` | `docker run … <image> transcribe /data/jfk.wav --format json` with the volume and the fixtures mounted gives the JFK sentence with `bias_scope: "public"` | PR-31 |

### External backend

| Test | Asserts | PR |
|---|---|---|
| backend cases | loopback, a remote `https:` backend, a remote `http:` backend (MSG-REMOTE-HTTP once per backend URL), and a keyed backend with no **User Action Key** (MSG-REMOTE-NO-KEY in the variant O-17 chose, install disabled before the click) | PR-29 |
| laptop policy | a staged managed policy with `dictation.enabled: false` or `dictation.capture: false` makes `isAllowedRendererPermission` refuse `media` and disables the clipboard switch | PR-29 |
| path table | a Rust test reads the path table in `utils/managedDictationPolicy.ts` and compares it with `Paths::managed_policy_path` on each OS | PR-29 |
| disclosure | Settings shows MSG-DISCLOSE-REMOTE with the backend's host | PR-29 |

### Managed policy and Copilot

| Test | Asserts | PR |
|---|---|---|
| policy tables (`managed::`) | absent; `enabled: false`; `capture: false`; each narrowing key; `true` equals absent; an unknown key, a wrong type, a failed load and an untrusted owner each turn dictation off with one warning naming the key (O-27) | PR-05 |
| parse isolation | a malformed `dictation:` block leaves `permissions.deny` in force (P-15) | PR-05 |
| `call_tool` managed deny | `POST /agent/call_tool` refuses a tool the managed policy denies | PR-05 |
| Copilot | a control action on Biorouter's own window is refused with MSG-COPILOT-SELF; observation is unchanged; `press_key` with ⌘⇧D or Ctrl+Shift+D is refused on another app's window | PR-07 |

`cargo test -p biorouter --lib managed::` passes on all three operating systems before PR-05 merges.

## The privacy test suite

Each privacy invariant is held by named tests. [Dictionary](dictionary.md) states the invariants and why; this table is where they are proved. A pull request that weakens one of these tests changes an invariant and needs the owner.

| Id | Invariant | Named tests | PR | Runs in |
|---|---|---|---|---|
| P-1 | Audio stays in memory in Biorouter's processes on the transcribing host: no temporary file, log line, diagnostics entry or session row; the stream buffer is zeroed at finish, cancel or the 60 s expiry; no off machine engine in v1. A reverse proxy in front of `serve` is the operator's: `browser-access.md` names the setting that keeps request bodies in memory (`client_body_buffer_size 512k` or `proxy_request_buffering off` on nginx), and M-17 records each proxy's equivalent | `dictation_routes.rs` (no new file under the sandbox root; a tracing capture holds no audio derived content); a unit test that the stream buffer is zeroed on drop; the Jupyter handler test (no file under `TMPDIR`); `no_cloud_audio_routes.rs`; the `serve` job's nginx log check | PR-01, PR-15, PR-21, PR-32 | offline, `serve`, `jupyter-ai.yml` |
| P-2 | Transcripts live only in the composer: never in notifications, logs, `messages.metadata_json`, telemetry or the worker's stderr | the tracing capture in `dictation_routes.rs`; the `UserMessage.tsx` log grep test; notification tests in `dictationAnywhere.test.ts`; the OpenAPI diff shows no dictation field on `Message`; `log_filter_drops_long_lines` | PR-04, PR-09, PR-15, PR-28 | lib, offline, vitest |
| P-3 | One bias set feeds every channel | the `bias.rs` truth table; `eligible_entries_are_built_only_by_bias_set` | PR-14 | lib |
| P-4 | The daemon decides the target | `dictation_routes.rs`: a refused `session_id` gives public scope and an identical response; a `new_chat_provider` sent without the proof is ignored; a `new_chat` stream takes the tier the bind would produce (an `ollama` cloud tag, a remote `OLLAMA_HOST` and a private worker under a public lead give public scope) | PR-15 | offline |
| P-5 | Public by default: Crew, clipboard, the app preview and `dictate` without an admitted `--session` take private terms only for one recording a person confirms after MSG-PRIVATE-TERMS, never for an agent (O-8). Public only: Jupyter, ACP and `transcribe` without an admitted `--session` | `bias.rs` rows for `crew` and `external` with and without `include_private_terms`; `include_private_terms` ignored without the proof and private standing; the CLI tests for `transcribe`, `dictate --copy` and `--private-terms` (refused with MSG-PRIVATE-TERMS-REFUSED without a terminal and under `BIOROUTER_TERMINAL=1`); the Crew and clipboard renderer tests; the Jupyter handler passes no session | PR-14, PR-15, PR-25, PR-27, PR-28, PR-32 | lib, offline, vitest, `jupyter-ai.yml` |
| P-6 | No existence oracle | `dictionary_oracles.rs`; the echo oracle test in `dictation_routes.rs`; the caps per tier unit test; `transcribe --format json` carries `bias_scope` and no count | PR-13, PR-14, PR-15, PR-16 | lib, offline |
| P-7 | A cooperative agent, and any caller on a keyed daemon without the proof, never opens a microphone, transcribes for itself through a capture path, installs, removes or selects a model, accepts or dismisses suggestions, edits or deletes a person's row, changes a tier, raises `agent_writes` or turns learning on. Residuals, recorded in SD-16: on a keyless daemon any holder of the daemon secret, including an agent shell that recovers it, can install a pinned model and make the dictionary writes a `serve` browser may make on rows it can list (private ones on a host whose operator's provider is private); and an agent holding a Copilot grant on the person's web browser can press the microphone in a `serve` tab or the Jupyter AI chat | `dictation_keyed.rs`; the keyless refusals in `dictionary_no_user_key.rs`; `capture_consent_is_minted_in_one_file`; CLI refusals when `BIOROUTER_TERMINAL=1`; the capability's tool list is exactly the three tools; the boundary refusals at `call_tool` and in `execute_code`; the Copilot refusal tests | PR-07, PR-11, PR-13, PR-15, PR-23, PR-25, PR-28 | lib, offline |
| P-8 | The person is warned, then free | renderer tests for MSG-PUBLICIZE-WARN, MSG-EXPORT-PRIVATE, MSG-PRIVATE-TERMS, MSG-PRIVATE-DRAFT, MSG-PRIVATE-DRAFT-SWITCH, MSG-REMOTE-HTTP and MSG-TURBO-CPU; the `--private-terms` CLI test | PR-18, PR-19, PR-20, PR-25, PR-27, PR-28, PR-29 | vitest, offline |
| P-9 | Agent terms carry no aliases and are suggestions by default; the capability is absent from Crew scoped, app, `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; ACP only suggests; the extractor never runs in ACP | the capability's tests; the extractor's skip tests | PR-23, PR-24 | lib, offline |
| P-10 | The network facing daemon cannot record | the daemon binary contract in `dictation_routes_auth.rs`; `cargo tree -p biorouter-server -e normal,build` holds no `biorouter-dictation-capture`; on the binaries `cross-build-nightly` builds with the release command, `biorouterd __br-dictation-worker capture` exits 2 and `strings biorouterd \| grep -c libpulse` is 0; the same `strings` check in every package smoke | PR-25 | offline, `guards`, `cross-build-nightly`, packaging |
| P-11 | Only pinned bytes are parsed | the installer tests (a mismatch deletes, a `.partial` is never loaded, routes and commands take ids only); the self test refuses a model that is not pinned; a fresh hash after an mtime change | PR-09, PR-11 | lib |
| P-12 | The worker is a closed box | the environment scrub in `dictation_worker.rs`; the 300 byte log line test | PR-09, PR-10 | lib, offline |
| P-13 | Tier changes are the person's | `the_tier_proof_is_constructed_in_exactly_two_files`; the keyless tier refusal in `dictionary_no_user_key.rs` | PR-13, PR-16 | lib, offline |
| P-14 | No private entry text leaves the store except to a caller allowed to list it | the instructions snapshot holds no term text; a piped `list` withholds private rows; refusal snapshots name no term; the store inspector refuses tool reads | PR-13, PR-16, PR-23 | lib, offline |
| P-15 | Policy only narrows | the parse isolation test; `dictation_policy_routes.rs`; a resumed roster does not load `dictionary` under policy | PR-05, PR-15, PR-23 | lib, offline |

These tests prevent accidents by cooperative actors. A determined actor with a shell can still read `dictionary.json` (DR-17), unset `BIOROUTER_TERMINAL` or relocate the policy file, and no test here claims otherwise.

## CI jobs

| Job | Trigger | What dictation adds | Network | First PR |
|---|---|---|---|---|
| `rust.yml` `guards` | pull requests and pushes | `check-vendored-whisper-cpp.sh`; `check-dictation-fixtures.sh`; `check-dictation-build-prereqs.sh`; the word list check; the retired name guard (R-31) over the vendored text | none | PR-08 |
| `rust.yml` `cross-check` | pull requests and pushes | nothing new to configure: `cargo check` runs the engine's build script, so the C and C++ compile in both cross images [C] | crate downloads only | PR-09 |
| `rust.yml` `test`, lib step | pull requests and pushes; ubuntu, macOS, Windows MSVC | every lib test above; the MSVC engine build or the stub (M-3); the macOS self inspection test; `check-macos-deployment.sh` on the macOS leg's `biorouterd` | none | PR-02 |
| `rust.yml` `test`, offline step | pull requests and pushes; ubuntu | `dictation_engine_offline`, `dictation_worker`, `dictation_cli`, `dictation_routes`, `dictation_routes_auth`, `dictation_keyed`, `dictation_no_user_key`, `dictation_policy_routes`, `dictionary_routes`, `dictionary_routes_auth`, `dictionary_no_user_key`, `dictionary_oracles`, `no_cloud_audio_routes`; `dictation_accuracy`, `model_download_live` and `dictation_extractor_precision` are compiled and hold only `#[ignore]` tests; no exclusion lines | loopback only | PR-01 |
| `rust.yml` `serve` | pull requests and pushes | the app page microphone test (PR-03); Playwright Chromium and Firefox capture; the `nginx:stable` prefix container; 413 halving; the nginx temporary file check; the worklet build output check (PR-21) | loopback in the tests; browser and image downloads at setup | PR-03 |
| `rust.yml` `cross-build-nightly` | schedule and `workflow_dispatch` | the self test in the `debian:bullseye` boot step, plus one real model JFK run there with the Light model, cached by SHA-256 (M-2); `check-dictation-isa.sh`; the Windows import check; the runtime deps named refusal; the unchanged glibc floor; from PR-25, the P-10 check on the binaries this job's release command built; uploads the mingw `biorouter.exe` and `biorouterd.exe` for the next job | image pulls; one model download | PR-09 |
| `rust.yml` `windows-native-dictation` (new) | schedule and `workflow_dispatch`, `needs: cross-build-nightly`, `windows-latest` | the self test of the shipped mingw binaries run natively; from PR-25, `biorouter dictation devices --format json` (a hosted runner has no input device, so the expected answer is the honest "no microphone"); a real model run: `whisper-small.en-q8_0` installed from A-1's release (cached by SHA-256) under a `BIOROUTER_PATH_ROOT` whose path holds characters outside ASCII, then `transcribe jfk.wav` gives the JFK sentence (M-1). whisper.cpp converts UTF-8 model paths to UTF-16 only under `_MSC_VER` [R], so a mingw build that hands the path to whisper.cpp fails under such a folder; this run is the check, whichever way [engine and models](engine-and-models.md) loads the file | model download | PR-09 |
| `frontend.yml` `unit` | pull requests and pushes | every vitest and browser test above | none | PR-01 |
| `frontend.yml` `dictation-capture-electron` (new) | pull requests and pushes; `macos-latest`, like `preview-panel` | builds the renderer with the repository's Vite, packs it into `app.asar` with `@electron/asar`, and runs Electron over `file://` with `BIOROUTER_TEST_FAKE_MIC_WAV` set to a generated 440 Hz tone at amplitude 0.5. Asserts: the worklet loads from `app.asar` under the real content security policy; RMS 0.3535 ± 0.005 and 440 ± 2 Hz at 16 kHz; samples for 98 % or more of wall time; audio only granted; audio with video and a foreign document refused with `NotAllowedError`; a hidden window captures; tracks ended and the context closed after stop. The probe script is `ui/desktop/scripts/dictation-capture-e2e.mjs`, modelled on [`preview-panel-e2e.mjs`](../../ui/desktop/scripts/preview-panel-e2e.mjs) | none (Electron download at setup) | PR-17 |
| `dictation-accuracy.yml` (new) | nightly (small.en); weekly (turbo); `workflow_dispatch` with a `version` input (both default models; run name `Dictation accuracy v<version>`); pull requests that touch `crates/biorouter-dictation/` or `vendor/whisper.cpp/` (small.en) | Tier 1, below; the MP3 and M4A decoding tests | GitHub release downloads; `apt-get install ffmpeg` | PR-12 |
| `dictation-model-pins.yml` (new) | nightly and `workflow_dispatch` | each catalog `sha256` against GitHub's `digest` for its asset; `gh release verify-asset` | GitHub API | PR-11 |
| `jupyter-ai.yml` (new) | pull requests and pushes that touch `integrations/jupyter-ai/`; `workflow_dispatch` | `pytest-jupyter` handler tests, Jest tests, one Galata smoke | package installs at setup; the tests use a fake `biorouter` | PR-32 |
| Docker workflow (new; proposed name `docker-image.yml`) | nightly and `workflow_dispatch` only (O-24) | amd64 and arm64 image builds; the self test; no model in the image; `serve` with one Playwright dictation | image pulls; one model download (the Light model and the voice detector, cached by SHA-256) | PR-31 |
| `linux-gui-packages.yml`, `windows-gui-packages.yml` | as today | the self test after the existing `biorouterd --version` check | none for the test | PR-09 |
| `release-artifact-smoke.yml` | `workflow_dispatch` during a release (the native Windows publish gate) | the self test of the installed app's `biorouter.exe` and `biorouterd.exe` | release download | PR-09 |

**Cost.** In the build spike, the static whisper.cpp build took 27 s on Linux, 36 to 39 s on Windows and 30 s on macOS arm64, and the Rust engine crate 19 s, 22 s and 75 s [M]. `rust-cache` does not cache the output of a workspace member's build script, so the C and C++ rebuild on every run of every leg [M for the observation, I for the consequence]. The MSVC build adds about 1.5 to 3 minutes to the Windows leg [I]. The `test` job's budget is 90 minutes [C]. PR-09 records the measured additions. The cache budget is 10 GB. The only speech files any job caches are `whisper-small.en-q8_0` and the voice detector (about 265 MB, keyed by SHA-256), in four jobs: Tier 1, `cross-build-nightly`, `windows-native-dictation` and the Docker workflow, which downloads them for its M-16 run. If no two of them share an entry, that is about 1.06 GB of the 10 GB [I]. Tier 1 downloads turbo (574 MB) for its weekly run and does not cache it.

## Release guards and packaging checks

| Guard | Where | Fails when | PR |
|---|---|---|---|
| minimum macOS | `scripts/check-macos-deployment.sh` in `release.sh` `build_mac_backends` (both architectures) and `cmd_verify` (the packaged `Contents/Resources/bin`) | the minimum OS of `biorouter` or `biorouterd` is not 12.0 | PR-02 |
| microphone entitlement | `cmd_verify` and `smoke_mac`: `codesign -d --entitlements - --xml` | `com.apple.security.device.audio-input` is missing on the app, `Biorouter Helper.app`, `Contents/Resources/bin/biorouter` or `Contents/Resources/bin/biorouterd`. Today those four carry it only because `osx-sign` 1.3.3 applies its default set [M]; this guard holds it whatever the signing configuration becomes | PR-04 |
| usage string | `cmd_verify` and `smoke_mac`: `plutil -extract NSMicrophoneUsageDescription raw` on the app's `Contents/Info.plist` | the value differs from "Biorouter uses the microphone only while you dictate, and does not save the recording." (the shipped app shows Electron's default text today [M]) | PR-04 |
| self test in every artifact | `smoke_mac` (arm64; x64 under `arch -x86_64` with `ROSETTA_ADVERTISE_AVX=1` on a macOS 15 or later host), `smoke_deb`, `smoke_rpm`, `smoke_cli_packages` and `smoke_serve` in [`smoke-test-release-artifacts.sh`](../../scripts/smoke-test-release-artifacts.sh); both containers of [`build-cli-linux-packages.sh`](../../scripts/build-cli-linux-packages.sh) (`debian:bookworm-slim`, `rockylinux:9`); `linux-gui-packages.yml`; `windows-gui-packages.yml`; `release-artifact-smoke.yml`; the `cross-build-nightly` bullseye boot step | the line `{"selftest":"ok",…}` is missing, or the exit code is not 0 | PR-09 |
| old processors | `scripts/check-dictation-isa.sh` | under `qemu-x86_64 -cpu Nehalem`, `--version` of either binary does not exit 0, or the self test does not exit 20 with MSG-CPU | PR-09 |
| Linux link floor | [`check-linux-runtime-deps.sh`](../../scripts/check-linux-runtime-deps.sh) and the glibc floor (`GLIBC_MAX="2.31"` in `release.sh`) | `DT_NEEDED` leaves `BASE_LIBS`, a symbol needs more than `GLIBC_2.31`, or a `GLIBCXX` version appears; the named refusal for audio and C++ runtime libraries | PR-09 |
| Windows imports | the new `cross-build-nightly` step | a C++, OpenMP or pthread runtime DLL is imported | PR-09 |
| capture absent from the daemon | every package smoke; `cross-build-nightly` on its own release build; `check-dictation-build-prereqs.sh` in `guards` | `strings biorouterd \| grep -c libpulse` is not 0, `biorouterd __br-dictation-worker capture` does not exit 2, or `biorouter-server`'s dependency tree holds `biorouter-dictation-capture` (P-10) | PR-25 |
| no model in any artifact (R6) | `ui/desktop/scripts/prepare-platform-binaries.js`; `build-cli-linux-packages.sh`; the Docker workflow | a file under `ui/desktop/src/bin`, in either CLI package or in the image starts with the ggml file magic (`0x67676d6c`, the bytes `lmgg` on disk [R]); or `biorouter dictation status --format json` in a package smoke reports an installed model | PR-11 |
| no model embedded in a binary (R6) | `no_model_is_embedded`, a lib source test in the leaf crate | an `include_bytes!` or `include_str!` outside `#[cfg(test)]` in `crates/biorouter-dictation`, `crates/biorouter-dictation-capture` or `crates/biorouter/src/dictation` names a `.bin` file or anything under `vendor/whisper.cpp/source/models/`. A scan of the binaries for the ggml magic cannot do this job, because the loader itself holds that constant; the embedded `jfk.wav` of `bench` is speech, not a model, and stays allowed | PR-11 |
| binary size ceiling (R6) | every package smoke | `biorouter` or `biorouterd` grows past its size before PR-09 plus PR-09's measured growth plus 1 MiB. The ceilings live in one file beside the smoke script, and a pull request that grows other code raises them in the same change, with the reason | PR-11 |
| asset count | `release.sh` today [C `release.sh:285`] | anything but exactly 11 release assets; speech models are never app release assets, so `release_assets`, `verify_remote_release_assets` and both updaters stay untouched | unchanged |
| accuracy before publish (O-22) | `release.sh` `cmd_publish` gains `require_dictation_accuracy`, modelled on `require_fresh_windows_smoke` [C] | no successful `dictation-accuracy.yml` run named `Dictation accuracy v<version>` exists at the release's source commit. It lands with PR-12 but stays inert, with a warning, until the first Tier 1 run on a default model is green; from then on it applies to every release cut. It needs the evaluation asset, which A-1 publishes before PR-12 merges | PR-12 |

Two preconditions are checked by a person and dated in the checklist's measurement log: before the first app release that carries PR-09, M-4 has passed (a notarized, hardened runtime app launches on macOS 12 and 13 and transcribes once on Metal); before the first that carries PR-11, A-1's releases pass `gh release verify-asset`.

**Changing the engine pin (DD-3).** The pin is `4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad`. Moving it is a pull request of its own, with these steps in order:

1. In a full clone, prove that the new commit contains every commit in `pin.json`'s `contains` list, starting with the CVE-2026-110810 fix: `git merge-base --is-ancestor 60c0be6 <new commit>`. Stop if it fails.
2. Replace the pristine `vendor/whisper.cpp/source/` subset from the new commit, check that every file in `patches/` still applies to a copy, and regenerate `source-manifest.json`; `check-vendored-whisper-cpp.sh` and the retired name guard pass.
3. Build in the three cross images (`cross_linux`, `cross_windows`, Docker bookworm) and on both macOS architectures, then rerun every self test and `check-dictation-isa.sh` through a dispatched `rust.yml`.
4. Run Tier 1 with both default models on dispatch, and a benchmark subset by hand.
5. Before the release that carries the bump, run Tier 2 (the pin changed).
6. Update `pin.json` (`commit`, `ggml`, `date`, `contains`) and the `NOTICE` line; the `NOTICE` pin test fails until both agree.

## Fixtures and their size budget

Four files are committed, inside the vendored subset, so no second copy exists. They are whisper.cpp's own test assets at the pinned commit [M]:

| File (under `vendor/whisper.cpp/source/`) | Bytes | SHA-256 | Used for |
|---|---:|---|---|
| `models/for-tests-ggml-base.en.bin` | 586,836 | `1bc042ca584ff1895897e95bffb34ccf357be46c1fca97cf7fbe32f2060aa9e8` | the loader, the self test, the English vocabulary (BPE and token counts); zero tensors, so `whisper_full` returns 0 with empty text |
| `models/for-tests-ggml-tiny.bin` | 575,451 | `c486fb9f14a28b1c1dc252741a431646cc573450c900b9d9c406e10294aa01e6` | the multilingual vocabulary, which is turbo's; zero tensors |
| `models/for-tests-silero-v6.2.0-ggml.bin` | 885,098 | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` | the voice detector with real weights, byte identical to the production `silero-vad-v6.2.0` file |
| `samples/jfk.wav` | 352,078 | `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` | 11.0 s of 16 kHz mono speech for the detector, the plumbing and the real model check |
| Total | 2,399,463 | | 2.29 MiB |

**Budget.** `scripts/check-dictation-fixtures.sh` sums every tracked file under `vendor/whisper.cpp/source/models/`, `vendor/whisper.cpp/source/samples/` and any `crates/biorouter-dictation/tests/fixtures/`, and counts `*.wav`, `*.flac`, `*.mp3`, `*.m4a` and `*.ogg` as audio:

| Limit | Value | Today | Headroom |
|---|---:|---:|---:|
| all fixtures | 2.75 MiB (2,883,584 B) | 2,399,463 B | 484,121 B |
| one file | 1 MiB (1,048,576 B) | 885,098 B (the detector) | 163,478 B |
| audio | 0.75 MiB (786,432 B) | 352,078 B | 434,354 B |

It runs in `guards` and `just check-everything`. Its self test fails on a 1.1 MiB dummy.

**Generated at test time, never committed:**

| Signal | Parameters | Used by |
|---|---|---|
| tone | 440 Hz, amplitude 0.5 (RMS 0.35355), 2.5 s | the Electron job, the `serve` job, decode tests |
| digital silence | exact zeros, 5 s and 1.5 s | the detector, the CLI denial path |
| pink noise | seeded, 5 s | the detector |
| WAV variants | 44.1 kHz stereo 16 bit, 48 kHz float, 8 kHz mono | decode and resample tests |
| FLAC | the tone, written by a small FLAC writer in the test helpers (verbatim subframes) [I] | decode tests |
| clipped signal | the tone at amplitude 1.5, clipped | decode tests |
| short and long clips | 0.4 s and 40 s | `too_short`, the 25 s split |
| corrupt model header | a valid header with a huge tensor count | the loader refusal |
| degraded speech | pink noise at SNR 10 dB and a synthetic room with RT60 about 0.4 s, from the clean evaluation clips with a fixed seed | Tier 1 only |
| MP3 and M4A | the tone, encoded by `ffmpeg` | Tier 1 only |

**Rules.**

- `.gitattributes` gains `vendor/whisper.cpp/source/** -text`, `*.wav binary` and `*.flac binary`, beside today's `vendor/computer-use/source/** -text` [C]. The fixture hash test turns a CRLF or LFS pointer checkout into a named failure instead of a model that loads garbage.
- No tokenizer text is committed. For both vocabularies, `vocab.json`, `tokenizer.json` and the decoded vocabularies spell the former upstream's name as BPE pieces; `merges.txt` does so for the multilingual vocabulary that turbo uses, while the English `merges.txt` of small.en has no match [M]. Golden BPE ids are committed as numbers. The binary fixtures are safe because the retired name guard reads tracked text with `git grep -I`, which skips them [C, M].
- No real transcripts and no clinical audio are committed. Accuracy audio lives in release assets (Tier 1) or a private repository (Tier 2).
- `assets/english-words.zst` is product data, not a fixture, and sits outside the budget. Because it is compressed, `git grep -I` cannot see inside it, so the generator under `scripts/dictation/` has a check mode that regenerates it from its pinned source, compares the bytes, decompresses it and scans every line with the retired name guard's own patterns. That check runs in `guards`.
- Every golden table and term list in a test is screened by the retired name guard before commit, as `guards` does anyway.

## Accuracy

### What is known so far

Dated summary of the measurements the plan relies on, all taken on 2026-11-04 [M]. They are design measurements, not claims about the product; DD-95 keeps such claims out of user documentation, release notes, the announcement and the interface until the lab baseline exists.

| Finding | Evidence |
|---|---|
| On real speech, turbo wrote gene symbols correctly 91 to 92 % strict, small.en 70 to 75 % | MultiMed gene symbols, text prompt |
| On real drug names with the voice detector and a glossary, strict recall was 82 % for turbo and 79 % for small.en | PriMock57 subset |
| With timestamps off, small.en collapsed on 12 of 49 real utterances; with them on, 2 of 49 | real speech |
| Without the detector, turbo produced output on 15 or 16 of the 16 gaps and small.en on 2 to 8 of them in the standard conditions (0 with timestamps on and no prompt, 13 with a "Glossary:" frame); with it, 1 of 16 (turbo) and 0 of 16 (small.en). The gaps are 10 spans of room tone and 6 clips where only the other party bleeds in, and bracketed tags such as `[BLANK_AUDIO]` are not counted as output | PriMock57 gaps |
| The guarded fuzzy corrector made 45 false edits over 4,651 real sentences and turned NTRK into NRAS | real text |
| The prompt as BPE ids raised turbo from 80 to 93 % pooled strict and small.en from 85 to 90 % | synthetic speech only |
| The decoder bonus reached 72 % strict at 1,000 terms against 49 to 52 % for the same terms as a prompt, with 0 insertions on 75 probe clips | synthetic speech only |
| About 80 term occurrences per public set, so a difference under 5 points is noise unless a paired test says otherwise | both sets |

Neither the token prompt nor the decoder bonus has been run on real speech; that is M-9.

### Tier 1: networked nightly accuracy

`dictation-accuracy.yml` runs on `ubuntu-latest`, CPU only, and executes the `#[ignore]` tests that need a real model or an encoder:

```bash
cargo test -p biorouter-dictation --test dictation_engine_offline --test model_download_live -- --ignored
cargo test -p biorouter --features local-dictation --test dictation_accuracy -- --ignored
sudo apt-get install -y ffmpeg
cargo test -p biorouter-cli --test dictation_cli -- --ignored
```

`dictation_accuracy.rs` lives in `crates/biorouter/tests/`, not in the leaf crate, so it builds the dictionary's bias set through `bias::bias_set` like the product does: a leaf crate test cannot reach the core crate, and calling `EligibleEntries::new_filtered` directly would fail `eligible_entries_are_built_only_by_bias_set`. The last command covers the MP3 and M4A decoding cases and the agent shell rule on reversed speech (the reversal is made from `jfk.wav` at test time).

1. **Models.** The test installs each model through the leaf crate's own installer into `$RUNNER_TEMP/models`, which `actions/cache` restores by the catalog's SHA-256, and points `BIOROUTER_DICTATION_MODELS_DIR` at it. A cache miss therefore also exercises A-1's download path with the product's own pins.
2. **Evaluation set.** The PriMock57 subset: 60 utterances with 85 occurrences of 26 drug names, and 16 gaps where the recorded speaker is silent (10 of room tone, 6 where only the other party bleeds in), about 7 MB of FLAC, CC BY 4.0. A-1 publishes it before PR-12 merges as the `BaranziniLab/biorouter-models` release `eval-primock57-subset-r1` (a FLAC tarball, `ATTRIBUTION.md` and its SHA-256; clipped from PriMock57 at commit `cd2ac707`, cut at TextGrid boundaries ±250 ms), under the `eval-<name>-r<n>` tag scheme with its own `catalog/eval-*.json` beside the `stt-` releases (DD-16). The test pins its SHA-256 and reads it from `BIOROUTER_DICTATION_EVALSET`, a variable only this test reads.
3. **Arms.** Each model runs twice, both times with the voice detector, which is always on: with no dictionary (the baseline arm), and with the subset's 26 terms as the dictionary through the shipped mechanism (the DD-44 default of the commit under test). Each arm runs on the clean utterances and on the seeded SNR 10 and reverb copies the test generates; the 16 gaps run as recorded.
4. **Scoring.** Strict recall is the share of reference term occurrences whose canonical written form appears exactly in the output. A collapse is an output of 25 % or less of the reference's words. A false correction is a change in the transcript's `changes` list that turns a correct reference word into another. A spurious insertion is a dictionary term in the output where the reference has none (the 16 gaps included).
5. **Floors.** On the clean utterances with the dictionary: strict recall 70 % or more for `whisper-small.en-q8_0` and 75 % or more for `whisper-large-v3-turbo-q5_0`. On the 16 gaps, in both arms: nonspeech outputs 0 of 16 (turbo at most 1 of 16). On every condition: 0 collapses; 0 false corrections; spurious insertions with the dictionary no higher than in the baseline arm. These are the starting floors; the first run with the shipped mechanism sets them again, and the new values are written into the test with the date. Recall on the degraded copies is reported, not gated, until the lab baseline [I].
6. **Output.** Per term tables, the full transcripts and the counts are uploaded as artifacts. The job fails only on a default model's floor; anything else is a `::warning::` annotation.

Schedule: small.en nightly and on pull requests that touch the engine; turbo weekly and on dispatch. One model's run is 2 arms of 60 utterances in 3 conditions plus the 16 gaps, about 392 decodes. At the measured 0.73 s (small.en q8_0) and 9.0 to 9.6 s (turbo) per utterance on 4 CPU threads [M], that is about 5 minutes for small.en and about an hour for turbo on comparable hardware; hosted runners are slower [I], so the turbo job's timeout is 240 minutes [I]. PR-12 lands the workflow with the baseline arm's floors (nonspeech, collapses) and records the no dictionary baseline; PR-14 adds the dictionary arm and its floors.

### Tier 2: the lab set (O-23)

Before each release that changes the engine, a model pin or a recognition default, the maintainer runs the private lab set on every candidate model, on Metal and on the CPU, and writes dated tables into [speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md).

The set, as recommended for O-23:

| Part | Content |
|---|---|
| Governance | an IRB determination first; one page consent with tiers: T1 internal evaluation on lab machines, T2 storage in the private repository and use by private CI, T3 (optional) up to 3 clips public under CC BY 4.0; pseudonymous speakers; no PHI, enforced by term based scripts, neutral prompt cards and a second listener who rejects any clip with PHI or third party names; withdrawal deletes the speaker's assets and cuts a new version |
| Speakers and rooms | 3 to 5 lab members across at least 3 accent groups; primary condition their own laptop microphone at their desk, recorded through Biorouter's own capture path; secondary conditions (about 20 sentences each, 2 speakers or more): a Bluetooth headset, lab bench noise, an open office; one speaker records 20 sentences twice, with browser audio processing on and off |
| Script | 60 read sentences: 40 dictation sentences with 1 to 3 terms each, every term at least twice, at the start, middle and end; 10 controls with no terms; 10 near homophone sentences from the collision list (spoke, crisper, router, PhD two, UCSD, smoke, here's, clinic, continue and others). Per speaker also 15 semi spontaneous prompts, 3 long recordings of 40 to 60 s, and 5 nonspeech clips of 5 s |
| Term list | about 60 entries: lab and product names (Biorouter, UCSF, Baranzini, SPOKE, OMOP), genes (EPAS1, EGLN1, PHD2, VHL, HLA-DRB1*15:01, IL7R, TYK2), MS drugs (natalizumab, ocrelizumab, ofatumumab and others), assays (`scRNA-seq`, `ATAC-seq`, CRISPRi, GWAS, eQTL, UMAP), and the collision controls |
| Storage | private `BaranziniLab/biorouter-dictation-evalset`: git holds `manifest.jsonl`, transcripts, `CONSENT.md`, `CONVENTIONS.md` and the term list; audio is private release assets under immutable tags (`evalset-v1.0`), each a per speaker FLAC tarball with its SHA-256 in the manifest; every text runs through the retired name guard before commit |
| Scoring | strict and lenient recall per term and category; insertions on controls; false edits on near homophones, from the corrector and from the prompt; WER with fillers dropped; term error rate; collapses; terms deleted at the start of an utterance; nonspeech output and echo rates; latency per hardware class. Conditions are compared with a paired sign test at the occurrence level; intervals come from a bootstrap clustered by speaker |

### The real speech regression gate

A release that changes the engine, a pin or a recognition default is published only when Tier 2 shows, at fixed model and settings against the previous dated table:

1. no strict recall regression of more than 3 points on the read set;
2. 0 nonspeech outputs with voice detection;
3. 0 false edits on the near homophone sentences;
4. 0 collapses.

The first lab run sets the baseline and no comparison applies. Tier 1's floors hold between releases.

### The M-9 decision and its fallback (DD-44)

M-9 runs on the lab set: none, token prompt, decoder bonus, both, and each with correction, on turbo and small.en; `temperature_inc` 0 against 0.2; timestamps on and off for small.en; browser audio processing on and off. The rules below are fixed before the numbers exist, because DD-44 rejects choosing after seeing them:

1. **Channels.** The combination of token prompt and decoder bonus stays the default only if, on both models, its strict recall on the read set is no more than 3 points below either channel alone (the same noise margin as the regression gate), a paired sign test does not find it worse than either at p < 0.05, and its counts of nonspeech outputs, insertions on controls and probe clips, and collapses are no higher than either channel's. Otherwise the declared fallback ships: the token prompt only, with the decoder bonus built but off.
2. **Other settings.** For `temperature_inc`, timestamps and browser audio processing, the current default stays unless the alternative passes the same test and is strictly better on at least one gate metric.
3. **Record.** The result, the arms and the date go into [speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md); a changed default lands in its own pull request with Tier 1 floors set again.

### Long dictation (DD-50, M-19)

The split rule (ask for voice segments once 8 s are buffered; queue closed segments of 25 s or less split at the longest pause; `no_context = true`) has unit tests but no measurement on real continuous speech. M-19 measures it through the real streaming path: an `#[ignore]` test in `dictation_routes.rs`, `long_dictation_replays_in_real_time`, starts the routes in process with a real model, replays each recording in 100 ms chunks batched every 2.5 s at real time pace, finishes, and records the Stop to text time and the boundary word loss (reference words within 1 s of a split that are missing from the output). Recordings: the lab set's long recordings and continuous PriMock57 doctor channels of 60 to 180 s. It runs on Metal and on an x86 CPU. The result is recorded and blocks announcing v1; a boundary loss clearly above the deletion rate inside segments reopens DD-50 [I].

### What blocks publishing (O-22)

A red Tier 1 run on a default model's floor blocks `release.sh publish` through `require_dictation_accuracy`. A red result on anything else (a degraded copy, a per term table) is reported and blocks nothing. Tier 2's regression gate is checked by the maintainer before the draft is made, and its dated table is linked from the release notes.

## The manual QA checklist per surface

PR-33 creates `docs/releases/dictation-test-checklist.md` and its row in the [releases index](../releases/README.md). It follows the shape of `docs/releases/privacy-and-workspace-test-checklist.md` [C]: every row says what to do, what should happen and what a failure looks like, because several of these fail by doing nothing visible. The "Voice dictation (Whisper)" line of the [CLI checklist](../cli/qa-checklist.md) calls dictation GUI only, which R8 contradicts [C]. PR-01 turns it into a pointer to a page that exists when PR-01 merges, this section of `docs/dictation/testing-and-release.md`, so the line never links a file that does not exist yet (`python3 scripts/docs-lint.py` reports such a link [C]); PR-33 points the line at the new checklist.

The checklist holds only what a hosted runner cannot do: a real microphone, macOS permission prompts and denials, Safari, a person's terminal app and tmux, a real proxy, WSLg, GNOME and KDE Wayland clipboards, and a signed, notarized app on macOS 12. A row that a CI job already covers names the job, so a reader knows the row checks the real device or account, not the logic.

The file is laid out as follows:

1. The context header.
2. **Before you start**: note the build (version, platform, architecture, signed, notarized); use an installed, signed build, never a dev build, except where a row says "dev app"; use a fresh OS account, or run `tccutil reset Microphone com.electron.biorouter`; configure two providers, one private (Llama Server, or a private Versa model) and one public; keep privacy tiers on; have a built in microphone and a Bluetooth headset; add `UCSF` and `EPAS1` as public entries and `EGLN1` from a private chat (so private); keep a short recording `memo.m4a` that says those terms.
3. One section per surface, S1 to S20, each a table with the columns **#**, **Do this**, **Expect**, **Failure looks like** and **CI covers**.
4. **Measurement log**: one row per M id with the date, the build, the machine, the result and who took it (O-29).

The key rows of each section follow. Messages are quoted by id; their text is in [surfaces](surfaces.md).

### S1 desktop app, macOS Apple Silicon

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S1.1 | On a fresh account, open a chat and click the microphone left of **Send**. | A muted microphone; the setup popover with MSG-SETUP, **Install**, **Not now**, **More in Settings**, and **Suggest dictionary words from my chats** ticked. Nothing downloads. | A download starts on the click; no microphone. | `DictationButton.test.tsx` |
| S1.2 | Press **Install**; quit at about half; reopen and press **Install** again. | Progress in the ring; the second install resumes; **Settings > Models > Speech models** shows `whisper-large-v3-turbo-q5_0` in use, the folder `~/.local/share/biorouter/dictation/models/` and the account; the Components group opens once and shows **Dictionary**; no toast. | The download restarts from zero; files elsewhere; no **Dictionary** row. | installer tests |
| S1.3 | Click the microphone. | macOS asks with "Biorouter uses the microphone only while you dictate, and does not save the recording." | Electron's default text; no prompt and silence. | `forgeMacUsage.test.ts`, `cmd_verify` |
| S1.4 | Allow, say "Ask SPOKE which drugs target EPAS1 at UCSF", click again; then press ⌘Z once. | The text appears at the caret with `EPAS1` and `UCSF` spelled as in the dictionary; nothing is sent; one ⌘Z removes exactly the dictated words. | Sent without Enter; inserted into another field; ⌘Z removes more or less. | Electron job, the dev app run |
| S1.5 | Run `tccutil reset Microphone com.electron.biorouter`, relaunch, click, and choose Don't Allow. | MSG-MIC-DENIED-MAC; **Open settings** opens Privacy & Security > Microphone; after allowing and restarting, dictation works. Record the error name and the TCC log line (M-6). | A silent failure; a generic error; the wrong pane. | none |
| S1.6 | Tap ⌘⇧D; hold it a second while speaking; press Escape during a recording; press Enter during a recording. | A tap toggles; a hold records while held; Escape discards; Enter stops and inserts and does not send. | Enter sends; Escape inserts. | `DictationButton.test.tsx` |
| S1.7 | Record, then lock the screen. Record again and choose **Restart & update**. | The words land in the draft on lock; the update asks MSG-RESTART-BUSY. | Speech lost; a restart without asking. | none |
| S1.8 | In a chat on the private provider, dictate "EGLN1", then switch the chat to the public model. In a chat on the public model, dictate "EGLN1". | MSG-PRIVATE-DRAFT-SWITCH, and **Switch anyway** proceeds. In the public chat the private entry never shapes the spelling. | No warning; a private entry shapes a public chat's dictation. | `ChatInput.dictation.test.tsx`, `bias.rs` |
| S1.9 | On a macOS 12 virtual machine, launch the notarized build, install the Light model and dictate once; repeat on macOS 13. | The app starts and transcribes on Metal (M-4). | dyld refuses the app; the worker crashes. | `check-macos-deployment.sh` |

### S2 desktop app, macOS Intel

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S2.1 | On an Intel Mac with AVX2, fewer than 16 cores and no `bench` record, open the setup popover. | `whisper-small.en-q8_0` (264 MB) is the offered default; turbo is marked `too_slow`. | Turbo offered by default. | suitability tests |
| S2.2 | In Settings press **Test speed**, then **Install** turbo. | MSG-TURBO-CPU with the measured time; **Install anyway** works. Record both models' real time factor at 4 and 8 threads (M-10). | Turbo installs without a warning. | `speechModelStore.test.ts` |
| S2.3 | On Apple Silicon with macOS 15 or later, install the Intel build and dictate. | Dictation works under Rosetta. | Exit 20; a crash. | `smoke_mac` x64 |
| S2.4 | On Apple Silicon with macOS 12 to 14 (any version before 15), open the Intel build's popover. | MSG-CPU-ROSETTA; no download offered. | A download, then a failure. | `src/cpu.rs` table |
| S2.5 | On a processor without AVX2 (a 2013 Mac Pro, or a VM with a masked CPU model). | MSG-CPU, then MSG-CPU-DESKTOP, before any download. | A download, then a crash. | `check-dictation-isa.sh` |

### S3 desktop app, Windows x64

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S3.1 | Install with `Biorouter-Setup-<ver>.exe` and install the model. | Files under `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\`, the local data folder PR-06 adds (DD-19), not under `%APPDATA%\UCSF\biorouter\data\` (Roaming, where Biorouter's other data lives at `main` [C]) or `app-<version>`. | Files in Roaming or inside the app folder. | path agreement test |
| S3.2 | Turn off "Let desktop apps access your microphone" and click. | MSG-MIC-DENIED-WIN; **Open settings** opens `ms-settings:privacy-microphone`. Record which `getMediaAccessStatus` value the switch produces on Windows 11 (`denied` or `restricted`). | Silence; a generic error. | `micErrors.test.ts` |
| S3.3 | Let another app hold the microphone exclusively and click; later unplug a USB headset while recording. | MSG-MIC-BUSY; unplugging stops and inserts what was heard. | A hang; lost speech. | none |
| S3.4 | Update through the installer; then run the zip build. | The model stays installed and in use in both. | The update removes the model. | none |
| S3.5 | Use Ctrl+Shift+D; use the tray's **Dictate to clipboard** with the window minimized; then close the last window. | Both work while a window exists; MSG-WINDOW-OPEN under the switch; closing the last window quits. | The tray item does nothing while minimized. | `dictationAnywhere.test.ts` |
| S3.6 | On an account whose name holds characters outside ASCII, install and dictate. | It works. | The model fails to load. | `windows-native-dictation` |

### S4 desktop app, Linux deb and rpm

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S4.1 | Ubuntu 24.04 (PipeWire), Ubuntu 22.04 (PulseAudio), Fedora (rpm): install, install the model, dictate. | No OS prompt; text inserted. Record throughput on each (M-7). | Silence; dropped audio. | Electron job (macOS only) |
| S4.2 | With no microphone; then with the microphone held by another app. | MSG-NO-DEVICE; MSG-MIC-BUSY. | A spinner that never ends. | `micErrors.test.ts` |
| S4.3 | On GNOME Wayland open **Dictate into other apps**. | MSG-GNOME in place of the shortcut picker; the tray item works with the AppIndicator extension. | A shortcut that registers and never fires. | none |
| S4.4 | As root run `biorouter dictation install --system`; remove the person's copy. | Status shows `location: system`; dictation works. | The system copy is not found. | lookup order tests |
| S4.5 | Upgrade the package. | The model stays. | The upgrade removes it. | R6 guard |

### S5 `biorouter serve`, browser on the same machine

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S5.1 | Start `biorouter serve` with no model. | The banner says "Dictation: no speech model; run `biorouter dictation install`". | No dictation line. | banner snapshots |
| S5.2 | Open the URL in Chrome and install from Settings. | The text says the download goes to the server; MSG-DISCLOSE-HOST on first use; the model lands in the host account's folder; **Show in Finder** is absent. | "On this computer" wording; a Finder button. | `serve` job |
| S5.3 | Dictate in Chrome, Firefox and Safari 27 at `http://127.0.0.1:8765`; deny once in each. | Text inserted; a denial shows MSG-MIC-BROWSER. Record Safari (M-8). | Safari never asks. | `serve` job (Chromium, Firefox) |
| S5.4 | On **Dictionary**, open the menu of a private row; then open **Add** on a host whose operator's provider is public. | **Make public…** is disabled with MSG-DICTIONARY-HOST-TIER; **Only in private chats** is disabled with MSG-DICTIONARY-HOST-PRIVATE; raising **Agent's terms** is disabled with MSG-DICTIONARY-HOST-SETTINGS. | The click fails after the fact. | vitest |
| S5.5 | Restart with `biorouter serve --no-dictation-install`. | Install refused with MSG-INSTALL-LOCKED; an installed model still works. | Install proceeds. | `dictation_no_user_key.rs` |
| S5.6 | Press ⌘⇧D in the served composer in each browser. | Record whether the page receives the chord and which tap durations register (M-18). | n/a | none |

### S6 `serve` behind an HTTPS reverse proxy or a JupyterHub prefix

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S6.1 | Behind nginx over HTTPS with HTTP/2 at `/user/<name>/biorouter/`, dictate. | MSG-SHARED-ORIGIN once before the first recording; the worklet loads; text inserted. | The microphone fails on the first click everywhere. | `serve` job (nginx prefix) |
| S6.2 | Set `client_max_body_size 32k`, dictate; then set it to `128k`. | At 32k, MSG-413; at 128k, dictation works with smaller chunks. | A silent failure. | `serve` job (413 halving) |
| S6.3 | Repeat S6.1 behind `configurable-http-proxy` and `jupyter-server-proxy`. | Record each proxy's result and the setting that keeps request bodies off its disk (M-17). | n/a | none |
| S6.4 | Have four people dictate at once on one host. | All finish; MSG-SERVER-BUSY under load, then a retry; MSG-SERVER-MEMORY when memory is short. | A stream that never finishes. | `dictation_routes.rs` |
| S6.5 | Two `serve` tabs in one browser while a reply streams, then dictate in the second. | Record PUT timings; MSG-WAIT appears if a PUT is held (M-5). | n/a | transport tests |
| S6.6 | On an iPhone in Safari and an Android phone in Chrome, open the HTTPS `serve` address; dictate once, then deny once. | Text inserted; the denial shows MSG-MIC-BROWSER (M-8). | No prompt; a dead microphone. | none |

### S7 `serve` at a plain HTTP network address

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S7.1 | Start `biorouter serve --host 0.0.0.0` and open `http://<LAN address>:8765`. | The existing refusal text, unchanged; the banner prints MSG-SERVE-TUNNEL, not a plain HTTP LAN URL. | A composer with a dead microphone; a LAN URL in the banner. | banner snapshots |
| S7.2 | Run `ssh -N -L 8765:127.0.0.1:8765 <host>` and open the address `serve` printed, with its `?t=` part. | Dictation works as in S5. | n/a | none |
| S7.3 | Behind a proxy that rewrites `Host` and serves plain HTTP. | The composer shows MSG-INSECURE and the microphone is unavailable. | A microphone that fails on click. | `dictationStore.test.ts` |

### S8 CLI only Linux deb and rpm on a server

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S8.1 | On a server whose systemd unit runs as `biorouter`, run `sudo -u biorouter -H biorouter dictation install`. | `dictation status` shows account `biorouter` and `~biorouter/.local/share/biorouter/dictation/models/`. | The model installs under root. | package smoke |
| S8.2 | Run `biorouter dictation install` as another user. | MSG-OTHER-ACCOUNT. | A model the service never sees. | none |
| S8.3 | Run `sudo biorouter dictation install --system`. | `/var/lib/biorouter/dictation/models/`, folders 0755 and files 0644; the service finds it. | Files readable only by root. | none |
| S8.4 | Run `biorouter dictate` on the server over SSH, then at its local console. | Over SSH: MSG-LINUX-NOAUDIO, then MSG-SSH-SERVICE when the `biorouter serve` service runs (MSG-SSH-ROUTES otherwise); at the console: MSG-LINUX-NOAUDIO, then MSG-LINUX-ROUTES; exit 2 and no device opened in both. | ALSA noise on the terminal; a recording of silence. | environment tests |
| S8.5 | Run `biorouter transcribe memo.m4a`. | The text on stdout. | n/a | `dictation_cli.rs` |
| S8.6 | On a VM with a masked CPU model. | MSG-CPU-SERVER, then MSG-CPU-VM. Run the census command on lab hosts (M-11). | A crash with code 132. | `check-dictation-isa.sh` |

### S9 interactive CLI on a local machine

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S9.1 | In the TUI press Ctrl+T, speak, press Enter. | `◌ Starting microphone…`, then `● REC` with the timer and level bars, then `◌ Transcribing`; the text at the cursor; not sent. | A literal `t` typed; the text sent. | `draw_input` snapshots |
| S9.2 | Repeat in the classic REPL, the shared daemon prompt, and `biorouter session attach` with `/dictate`. | The REPL dictates instead of transposing; `attach` asks `Send? [y/N]` and defaults to No. | A transposition; an automatic send. | pseudo terminal test |
| S9.3 | On macOS try Terminal, iTerm2, Ghostty, VS Code's terminal and tmux; deny in one. | The prompt names the terminal app; a denial shows MSG-TCC-ZERO naming it. Record tmux attribution. | Silence with no message. | controller tests |
| S9.4 | On Windows try Windows Terminal and conhost with the privacy switch off. | MSG-WIN-ZERO. | Silence. | controller tests |
| S9.5 | Run `biorouter dictation test --seconds 10` on macOS, Windows 11, Ubuntu 24.04 (PipeWire) and 22.04 (PulseAudio). | Frames within 2 % of wall time; the first frame in under 300 ms (M-7). | Dropped frames; a long start. | none |
| S9.6 | Press Ctrl+C during a recording while a reply streams. | Only the dictation is cancelled. | The reply is cancelled. | `App` unit tests |
| S9.7 | On macOS, dictate from a terminal app whose `Info.plist` lacks `NSMicrophoneUsageDescription`. | No prompt ever appears; MSG-TCC-ZERO with its sentence for a terminal that never asks. | Silence with no message. | controller tests |
| S9.8 | Dictate with Ctrl+T in the desktop app's terminal dock. | It records; record which app the TCC prompt names. | n/a | none |

### S10 CLI over SSH or in a container with no microphone

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S10.1 | `ssh <Linux host>`, then `biorouter dictate`, with and without a model installed there. | MSG-SSH-ROUTES (MSG-SSH-SERVICE when a `biorouter serve` runs there); exit 2; no device opened; no install prompt when no model is installed. | A recording of silence; an offer to download a model. | environment tests |
| S10.2 | `ssh <Mac>`, then `biorouter dictate`. | MSG-SSH-MAC, then the routes. | A TCC prompt on the remote screen. | environment tests |
| S10.3 | `docker run -it <image> dictate`. | MSG-CONTAINER. | ALSA noise. | package smoke |
| S10.4 | Follow the tunnel route, opening the address `serve` printed with its `?t=` part; then run `biorouter dictionary export - \| ssh <host> biorouter dictionary import -`. | The laptop's browser dictates with the host's model and dictionary; the terms appear on the host. | n/a | CLI import tests |
| S10.5 | Run `biorouter serve` in a VS Code Remote SSH terminal and in Cursor. | Record whether each forwards the port and opens a working address. | n/a | none |

### S11 noninteractive CLI

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S11.1 | `biorouter transcribe memo.m4a --format json`. | Schema 1 with `bias_scope: "public"`. | A count of entries in the output. | `dictation_cli.rs` |
| S11.2 | `biorouter run -t "$(biorouter dictate)"`. | The dictated text is shown for review before `run` starts. | `run` starts with unreviewed text. | review test |
| S11.3 | Ask the agent in a chat to run `biorouter dictate` in its shell. | MSG-AGENT-SHELL; exit 2. | A microphone opens. | `capture_consent_is_minted_in_one_file` |
| S11.4 | Run `biorouter dictate < /dev/null` from a script. | MSG-NO-TTY; exit 2. | A recording starts. | environment tests |
| S11.5 | `biorouter dictate \| biorouter crew send <channel> --input -`. | Reviewed on the terminal before anything is posted. | A post without review. | review test |
| S11.6 | `biorouter crew ask <channel> "$(biorouter dictate)"`. | Reviewed on the terminal before the task starts. | A task starts with unreviewed text. | review test |

### S12 ACP clients

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S12.1 | In Zed and in a JetBrains IDE (stdio), look at the agent chat. | No microphone and no dead control. | n/a | `on_initialize` test |
| S12.2 | Send an audio block from any client that can. | MSG-ACP-NOTE in the turn. | The clip vanishes silently. | ACP audio test |
| S12.3 | Use the tray's **Dictate to clipboard** (or `biorouter dictate --copy`) and paste into the editor. | The text pastes, with public terms unless the person chose **Include my private terms** (or `--private-terms`) and confirmed MSG-PRIVATE-TERMS. | Private terms without the confirmation. | `bias.rs` |
| S12.4 | Ask the agent to add a dictionary term. | A suggestion, and the answer names `biorouter dictionary review`. | An active row. | capability tests |
| S12.5 | Connect a `--ws` client. | No dictionary tools offered. | Tools offered. | ACP `--ws` test |
| S12.6 | In Zed, open the agent's command list and run `/dictate`. | `/dictate` is listed and answers MSG-ACP-DICTATE without calling the provider. | No command; a model turn. | ACP `/dictate` test |

### S13 Jupyter AI persona chat

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S13.1 | In JupyterLab with Jupyter AI 3.0.x over HTTPS, click the chat's microphone, speak, click. | MSG-JUPYTER-RECORDING, then MSG-JUPYTER-TRANSCRIBING; the text at the caret; nothing sent. | Sent automatically; an `@Biorouter` mention added. | Galata smoke |
| S13.2 | Without a model; then without `biorouter` on `PATH`. | MSG-JUPYTER-NO-MODEL; MSG-JUPYTER-NO-BINARY. | A stack trace. | handler tests |
| S13.3 | At a plain HTTP network address. | MSG-JUPYTER-INSECURE. | A microphone that fails on click. | Jest |
| S13.4 | Click the microphone for the first time, on a JupyterHub path prefix and on a single JupyterLab. | MSG-SHARED-ORIGIN once before the first recording, in both. | No notice. | Jest |
| S13.5 | Dictate in Chrome, Firefox and Safari. | Each decodes and inserts (M-13). | One browser inserts nothing. | none |
| S13.6 | During a dictation, list the server's `TMPDIR` and read the server log. | No audio file; no audio or text in the log. | A WAV file; a transcript in the log. | handler tests |
| S13.7 | Make `biorouter transcribe` fail (for example, remove the model mid dictation). | MSG-JUPYTER-FAILED. | A stack trace; a silent failure. | handler tests |

### S14 Crew

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S14.1 | In a channel composer, hover the microphone. | MSG-CREW-TIP. | No tooltip. | `check-crew-manual.mjs` |
| S14.2 | Dictate "@agent summarize the thread". | The text in the box; nothing posted and no agent started until Enter. | An agent run starts. | Crew composer tests |
| S14.3 | Dictate "EGLN1" in the box; then choose **Include my private terms** from the microphone's context menu, choose **Include** in MSG-PRIVATE-TERMS, and dictate it again. | The first time the private entry never shapes the spelling; the second time it may, for that recording only. | Private terms without the confirmation; the choice remembered. | `bias.rs`, Crew composer tests |
| S14.4 | Use the **Ask my agent** box. | As S14.2. | As S14.2. | Crew composer tests |
| S14.5 | Open Crew in a `serve` browser. | "Crew needs the Biorouter desktop app". | A dead microphone. | none |

### S15 Agent Drafter apps

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S15.1 | Open a built app's page in a `serve` browser and call `getUserMedia` from its console in Chrome, Firefox and Safari. | Chrome refuses; record Firefox and Safari, the documented residual. | Chrome grants it. | `serve` job (Chromium) |
| S15.2 | In the desktop preview, click into an app's text field, then press **Dictate into this app**. | The text lands in the field (M-15); if M-15 failed, the button is absent. | Text in the wrong place. | none |
| S15.3 | Hover that button with no field focused. | MSG-APPS (if M-15 failed, the SDK reference carries MSG-APPS-NO-PREVIEW instead). | n/a | vitest |
| S15.4 | Open an exported app in a browser; use **Dictate to clipboard** and paste. | The text pastes. | n/a | none |

### S16 Docker image

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S16.1 | Run the documented `docker run -p 127.0.0.1:8765:8765 …` command, open `http://127.0.0.1:8765/?t=<token>` as MSG-SERVE-CONTAINER in the banner says, install from Settings and dictate. | The model lands in the data volume; dictation works (M-16). | An insecure context; the model inside the container layer. | Docker workflow |
| S16.2 | Recreate the container with the same volume. | The model is still installed. | A second download. | none |
| S16.3 | Mount a model folder read only at `/var/lib/biorouter/dictation/models`. | Found as `location: system`. | Not found. | lookup order tests |
| S16.4 | `docker run … <image> transcribe /data/memo.wav`. | The text. | n/a | Docker workflow |

### S17 desktop app against an external backend

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S17.1 | In Settings > App > **Use external server**, point at a LAN `biorouterd` behind nginx over HTTPS with its **User Action Key**; dictate in a private chat. | MSG-DISCLOSE-REMOTE; dictation works with private terms. Record latency and upload timings (M-20). | "On this computer" wording. | external backend tests |
| S17.2 | Remove the key. | MSG-REMOTE-NO-KEY in the O-17 variant; install disabled before the click. | Install fails after the click. | external backend tests |
| S17.3 | Point at a plain `http:` backend. | MSG-REMOTE-HTTP once per URL; **Continue** proceeds. | A refusal; a warning every time. | external backend tests |
| S17.4 | Stage a laptop managed policy with `dictation.capture: false`. | No microphone in any composer; the **Dictate into other apps** switch is disabled; Settings shows MSG-POLICY-CAPTURE; a page that calls `getUserMedia` gets `NotAllowedError`. | The backend's policy decides instead. | policy path test |
| S17.5 | Point at a `biorouterd` older than local dictation. | MSG-REMOTE-OLD on the microphone and the **Dictionary** page; their controls disabled. | A dead microphone; a raw 404. | external backend tests |

### S18 other applications (Dictate to clipboard)

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S18.1 | Turn on **Dictate into other apps**. | MSG-CLIPBOARD-EXPLAIN once; on macOS the microphone prompt. | No explanation. | `dictationAnywhere.test.ts` |
| S18.2 | From the tray choose **Dictate to clipboard**, speak, choose **Stop and copy**, paste into TextEdit, Word or Notepad. | MSG-CLIPBOARD-COPIED; the text pastes; the notification shows counts only. | Words in the notification. | `dictationAnywhere.test.ts` |
| S18.3 | Record silence. | MSG-CLIPBOARD-NOTHING; the clipboard is unchanged. | The clipboard is overwritten. | `dictationAnywhere.test.ts` |
| S18.4 | Choose a shortcut another app holds. | MSG-SHORTCUT-TAKEN. | A shortcut that never fires. | `dictationAnywhere.test.ts` |
| S18.5 | On native Windows with the cloud clipboard on, run `biorouter dictate --copy`. | The text does not reach another device. | It syncs. | clipboard helper tests |
| S18.6 | With the packaged deb on Ubuntu 26.04 GNOME, Fedora and KDE Plasma 6 (X11 and Wayland), and on a Windows 11 laptop. | Record hidden window capture with a real microphone, clipboard focus and the KDE portal shortcut (M-12). | n/a | none |
| S18.7 | `biorouter dictate --copy` over SSH with no display; then on a Linux desktop. | MSG-NO-CLIPBOARD and exit 5; MSG-CLIPBOARD-WAIT and a paste within 60 s works. | The text is lost when the CLI exits. | clipboard helper tests |
| S18.8 | Turn off **Show in menu bar** in Settings > App, then open **Dictate into other apps**. | MSG-NO-TRAY; a shortcut is required. | A switch that starts nothing. | `dictationAnywhere.test.ts` |
| S18.9 | On native Windows with the cloud clipboard on, use the tray's **Dictate to clipboard**. | MSG-CLIPBOARD-EXPLAIN was shown when the switch was turned on, and syncing to another device is expected: the desktop form cannot set the exclusion in Electron 39. | The switch turned on without MSG-CLIPBOARD-EXPLAIN. | `dictationAnywhere.test.ts` |

### S19 WSL

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S19.1 | In WSL2 with WSLg and `libpulse0`, run `biorouter dictate`. | It records and transcribes. | Silence. | environment tests |
| S19.2 | Remove `libpulse0`; run it again. | MSG-WSL2-NOPULSE. | A generic error. | environment tests |
| S19.3 | In WSL1. | MSG-WSL1. | A hang. | environment tests |
| S19.4 | `biorouter dictation install --from /mnt/c/Users/<you>/Downloads/`. | It installs by hash. | A second download. | `--from` tests |
| S19.5 | `biorouter dictate --copy`, then paste in a Windows app. | The text pastes through `Set-Clipboard`. | n/a | clipboard helper tests |
| S19.6 | Run `biorouter serve` in WSL and open the address it prints, with its `?t=` part, in a Windows browser (`localhost` in place of `127.0.0.1` works too). | Dictation works as in S5. | n/a | none |
| S19.7 | In WSL2 with `guiApplications=false` in `.wslconfig`, run `biorouter dictate`. | MSG-WSL2-NOWSLG; exit 2. | A generic error. | environment tests |

### S20 air gapped or offline hosts

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S20.1 | With the network off, run `biorouter dictation install`. | MSG-OFFLINE. | A hang. | installer tests |
| S20.2 | On a connected machine run `biorouter dictation urls`, download the files, carry them over and run `biorouter dictation install --from <dir>`. | Installed by hash, renamed files included; a wrong file gives MSG-HASH. | A renamed file refused. | `--from` tests |
| S20.3 | On the desktop, copy the files into the model folder (**Show in Finder**) and press **Rescan**. | The model is listed as installed. | n/a | rescan route test |
| S20.4 | Stage managed `model_downloads: false` with a `models_dir`. | Only that folder is used; nothing is offered for download. | A download starts. | `dictation_policy_routes.rs` |
| S20.5 | Dictate with the network off while watching outgoing connections. | It works and nothing connects out. | Any connection attempt. | none |

## Measurements still to take

Each measurement is recorded with its date in the pull request that takes it, and copied into the checklist's measurement log by PR-33. Announcing v1 needs M-1 to M-10, M-12, M-13 and M-15 to M-20 (DD-94, O-29): M-1 to M-10 and M-19 by name, and the others because checklist rows S5.6, S6.3, S13.5, S15.2, S16.1, S17.1 and S18.6 record them and every section must pass. M-11, M-14 and M-21 feed later decisions.

| Id | Measurement | How | Earliest | Blocks |
|---|---|---|---|---|
| M-1 | Native Windows run of the static mingw worker | nightly `windows-native-dictation`, the package self test, a real model JFK run under a path outside ASCII | the first nightly after PR-09 | announcing v1 |
| M-2 | Real x86 Linux (not Rosetta or qemu) | the self test and the JFK run in the `cross-build-nightly` bullseye container on a hosted amd64 runner; a real model run on a lab server | the first nightly after PR-09 | announcing v1 |
| M-3 | The engine under MSVC (`/MD`) | the Windows `test` leg in PR-09 | PR-09 | PR-09 merge (or the `no_engine` stub) |
| M-4 | A signed, notarized, engine linked app on macOS 12 and 13; Metal under the hardened runtime with the pinned entitlements; a first transcription | `release.sh mac-arm64`, a macOS 12 VM, checklist row S1.9. A follow up measurement found that Metal transcribed under the hardened runtime with `osx-sign`'s default entitlements, with none and with an empty set, cold shader compile included, on ad hoc signed copies [M]; the notarized app on macOS 12 and 13 is the unmeasured part | after PR-09 | the first app release that carries PR-09 |
| M-5 | Connection pool: two windows and two `serve` tabs while a turn streams; then behind HTTP/2 nginx | real app, real provider, DevTools PUT timings; rows S6.5 and a desktop twin | after PR-19 and PR-21 | announcing v1 |
| M-6 | The TCC denial's error name and message on a signed app | fresh user or VM, `tccutil reset`, `log stream --predicate 'subsystem == "com.apple.TCC"'`; row S1.5 | after PR-19 | announcing v1 |
| M-7 | Real microphones per OS family, app and CLI; PipeWire, PulseAudio and ALSA throughput at a 100 ms period; CLI time to first frame under 300 ms | rows S4.1 and S9.5; `biorouter dictation test --seconds 10` | PR-19 (app), PR-25 (CLI) | announcing v1 (PR-25 acceptance) |
| M-8 | Safari at `127.0.0.1` and behind HTTPS, including its first click and first ⌘⇧D before the status store has loaded (both must reach recording); iOS Safari; the `Permissions-Policy` feature check that decides MSG-SHARED-ORIGIN; the Firefox ESR fallback before 148 | rows S5.3, S6.1 and S6.6 | after PR-21 | announcing v1 |
| M-9 | The shipped combination on real speech, all arms | Tier 2 with the lab set; the rules in [The M-9 decision and its fallback](#the-m-9-decision-and-its-fallback-dd-44) | after PR-14 and O-23 | any accuracy claim; the DD-44 default; announcing v1 |
| M-10 | Native x86 latency and real speech accuracy of `whisper-small.en-q8_0`, `whisper-large-v3-turbo-q5_0` and `whisper-large-v3-turbo-q8_0` (the encoder window sized to the utterance, with the repetition guard) with the shipped flags, on an Intel Mac, a Windows laptop and a Linux server, 4 and 8 threads | `biorouter dictation bench`, Tier 2, row S2.2. Decision rule: it confirms q8_0 as the CPU default (DD-14) and decides whether `whisper-large-v3-turbo-q8_0` joins the catalog for hosts with 8 or more big cores; after a `bench`, turbo is `recommended` on a CPU host only at a real time factor of 0.5 or less (before one, DD-22's core count rule applies); a faster CPU tier opens for a later release if small.en's median exceeds 1.5 s per 5 s utterance on a lab laptop, or its real speech gene recall is below 75 % strict while turbo's factor there exceeds 0.5 | after PR-11 | per hardware defaults; announcing v1 |
| M-11 | Lab host ISA census | `grep -o 'avx2\|fma\|f16c\|bmi2' /proc/cpuinfo \| sort -u` on lab servers and hub nodes | any time | O-11 |
| M-12 | Hidden window capture with a real microphone on Windows 11 and PipeWire; Wayland clipboard focus; the KDE portal shortcut | row S18.6 | PR-28 | announcing **Dictate to clipboard** on Linux (PR-28 acceptance) |
| M-13 | Jupyter: `decodeAudioData` on Chrome WebM and Opus, Firefox Ogg and Opus, Safari MP4 and AAC; the caret after a programmatic `value` set | a Galata matrix; row S13.5 | PR-32 | announcing the Jupyter microphone |
| M-14 | `available_parallelism()` under a cgroup v2 quota; worker memory; four concurrent streams | a KubeSpawner pod; `bench` on a server; row S6.4 | after PR-15 | several decoder states in a later release |
| M-15 | `webContents.insertText` into the focused field of an app preview `WebContentsView` after the main renderer took focus | the dev app with a built example app; row S15.2 | PR-28 | shipping **Dictate into this app** |
| M-16 | Docker `serve` end to end | PR-31 acceptance; row S16.1 | PR-31 | the Docker documentation |
| M-17 | Proxies other than nginx (`configurable-http-proxy`, `jupyter-server-proxy`, `ingress-nginx`, Caddy, Traefik), and each one's setting that keeps request bodies off disk (P-1) | containers running the upload ladder of 64 KiB to 2 MiB; row S6.3 | after PR-21 | the proxy section of [browser access](../deployment/browser-access.md) |
| M-18 | Whether browsers let a page prevent ⌘⇧D; tap durations on the chord | row S5.6 | after PR-21 | the tooltip on `serve`; the 500 ms threshold |
| M-19 | Long dictation: 60 to 180 s of continuous real speech through the real streaming path; boundary word loss and Stop to text time on Metal and on an x86 CPU | `long_dictation_replays_in_real_time` with the lab set's long recordings and PriMock57 doctor channels | after PR-15 | announcing v1 |
| M-20 | External backend: latency and uploads to a LAN `biorouterd` behind nginx | row S17.1 | PR-29 | announcing S17 |
| M-21 | Extractor precision over the person written messages of 200 of the owner's sessions, offline and local | the procedure under [The capability and the extractor](#the-capability-and-the-extractor) | PR-24 and O-23 | the default of `learn_from_chats` (on only at 0.8 or more at `seen ≥ 2`) |

## Release sequence and announcement

### Releases before version 1

An app release may carry any prefix of PR-01 to PR-33. Nothing hides behind a release flag (DD-94), so what a release carries works, and its notes say so plainly without calling dictation finished. Releases keep exactly 11 assets; speech models never ship with an app release.

Release notes items, by the first release that carries each pull request:

| First release carrying | The notes say |
|---|---|
| PR-01 | the inherited cloud dictation is removed, including `/audio/transcribe` and `/audio/transcribe/elevenlabs`; a saved `ELEVENLABS_API_KEY` stays in the keyring, unused, and can be deleted by hand |
| PR-02 | `biorouter` and `biorouterd` declare macOS 12.0 as their minimum, which the app already required |
| PR-04 | the macOS microphone prompt names Biorouter's purpose |
| PR-09 | the measured growth of each binary; processors without AVX2, FMA, F16C and BMI2 cannot run local dictation, and Biorouter says so instead of crashing |
| PR-10 | `biorouter dictation status` and the **Dictation** section of `biorouter doctor` |
| PR-11 | speech models are optional and never downloaded on install or update; they come from `BaranziniLab/biorouter-models`, are checked against SHA-256 pins compiled into Biorouter, and install into Biorouter's own data folder; the `NOTICE` additions (whisper.cpp, miniaudio, speech models downloaded on request) |
| PR-12 | `biorouter transcribe` turns audio files into text on this computer |
| PR-13, PR-14 | the personal dictionary in the terminal, and its effect on `transcribe` |
| PR-19, PR-20 | the microphone beside **Send** and the **Dictionary** page in the desktop app |
| PR-21 | dictation in `biorouter serve` browsers, with `--no-dictation-install` |
| PR-23, PR-24 | the `dictionary` capability, off until the person turns it on, and whether chats suggest terms automatically, with the reason if not |
| PR-25 to PR-32 | each surface as it lands: terminal dictation, Crew, **Dictate to clipboard**, external backends, the ACP note, the Docker `serve` image, the Jupyter AI microphone |

### The announcement (PR-33, O-29)

Version 1 is announced only when all of these hold (DD-94):

1. PR-01 to PR-33 are merged.
2. M-1 to M-10, M-12, M-13 and M-15 to M-20 are recorded with dates in the measurement log (M-11, M-14 and M-21 feed later decisions).
3. A-1's releases pass `gh release verify-asset`, and `dictation-model-pins.yml` is green.
4. Every section S1 to S20 of `docs/releases/dictation-test-checklist.md` passes on its surface with a signed build of the release being announced.

PR-33 then updates the six documents of `docs/dictation/` from plan to record of what shipped, adds the checklist and its row in the [releases index](../releases/README.md), the rows in `docs/README.md` and `docs/organization.md`, the "What shipped" bullets in [privacy tiers](../security/privacy-tiers.md) (including the ACP harvesting residual), [data privacy and PHI](../security/data-privacy-and-phi.md), the [CLI README](../cli/README.md), [common problems and fixes](../troubleshooting/common-problems-and-fixes.md) (microphone permission on each OS, WSLg), a "Local dictation" block in `CLAUDE.md` (the worker marker, the build traps, the `?no-inline` trap, the fake permission switch trap, the minimum OS guard, the dictionary tier rule, where tests run, and the correction of the `/agent/call_tool` line), and `landing/docs.html`. Its acceptance is `scripts/verify-docs.sh`, `python3 scripts/docs-lint.py` and the retired name guard.

The announcement says what dictation does and where it runs on each surface, that models are opt in with their sizes and folders, how private dictionary terms are kept out of public chats (as accident prevention for cooperative actors, never as security), and what waits for a later release, as listed in [surfaces](surfaces.md#what-waits-for-a-later-release): ACP audio blocks, `br.dictate()` in app pages, a JupyterLab dictionary panel, hold to talk in the terminal, a GNOME Wayland global shortcut, push to talk and synthetic paste for **Dictate to clipboard**, and an off machine engine. It carries no accuracy figure except the dated lab tables.

### Rollback

| Situation | Action |
|---|---|
| An organization must stop dictation now | managed `dictation.enabled: false`, or `dictation.capture: false` for live microphones only; every door refuses with MSG-POLICY or MSG-POLICY-CAPTURE without a new build |
| A `serve` host must stop browser installs | `biorouter serve --no-dictation-install`, or `BIOROUTER_DICTATION_BROWSER_INSTALL=false` |
| A person wants dictation gone | **Remove** in Settings or `biorouter dictation remove <id>` (the voice detector goes with the last speech model); turn off **Suggest dictionary words from my chats** |
| A build must ship without the engine | build `biorouter-cli` and `biorouter-server` with the default `local-dictation` feature off (and the CLI's `dictation` feature); `dictation status` reports `compiled: false` and every surface shows MSG-NO-ENGINE |
| A surface regresses after release | revert the pull request that made it work and cut a patch release; there is no flag to flip |
| A published model file is wrong | releases are immutable, so publish a new `stt-<id>-r<n+1>` tag, pin it in an app release, and keep the old hash in `accepted_sha256` only if the old file was good; nothing updates an installed model automatically |
| An engine pin bump regresses | revert the bump; the previous pin's Tier 1 floors and Tier 2 table still hold |

## Related documentation

- [Implementation plan](implementation-plan.md): the decisions, pull requests and owner decisions this test plan proves.
- [Engine and models](engine-and-models.md): the engine, the worker, the build rules and the model catalog that the build, packaging and installer tests hold in place.
- [Dictionary](dictionary.md): the store, the bias set, the capability and the privacy invariants that the dictionary and route tests prove.
- [Surfaces](surfaces.md): the behavior and the exact message text that each checklist section expects.
- [Speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md): the measurements behind the floors, and where Tier 2's dated tables go.
- [Releases](../releases/README.md): the release folder where the dictation checklist joins the existing checklists.
- [CLI QA checklist](../cli/qa-checklist.md): the terminal checklist whose dictation line PR-01 points at this document and PR-33 at the new checklist.
- [Launching the dev GUI](../desktop-ui/launching-the-dev-gui.md): how to start the dev app from a shell for the Playwright runs, such as the one undo step check.
- [Browser access](../deployment/browser-access.md): the `serve` deployment that the S5 to S7 rows and the proxy measurements exercise.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules that the privacy test suite holds, and where the shipped behavior is recorded.
