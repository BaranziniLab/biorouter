# Dictation engine and models

> **What this is.** The detailed design of the engine half of local dictation: the vendored whisper.cpp engine and the worker process that runs it, the native build and its platform floors, CPU and GPU backends, the speech model catalog, hosting the models in `BaranziniLab/biorouter-models`, installing and removing them on every OS and deployment, the administrator's switch, the daemon's `/dictation/*` routes, and the removal of the inherited cloud dictation.
> **Status:** Current: the plan being built on branch `feat/local-dictation`, pull request by pull request in the order of the delivery plan
> **Audience:** developers who build PR-01, PR-02, PR-05, PR-06, PR-08 to PR-12 and PR-15, the reviewers of those pull requests, and the owner, whose decisions O-1, O-2, O-17, O-35, O-37 and O-38 of 2026-11-05 and the plan's defaults O-11 to O-13 and O-25 to O-27 this document follows, and who performs A-1.

The [implementation plan](implementation-plan.md) decides what dictation does and in what order it ships. This document says how the part between a buffer of 16 kHz audio and a corrected string is built: which code is vendored, how it is compiled into `biorouter` and `biorouterd`, how a worker process runs it, which models exist and where they come from, how a person installs one, and how the daemon accepts audio over HTTP. How the personal dictionary biases and corrects recognition is in [dictionary](dictionary.md). Capture, the buttons, and every sentence a person sees are in [surfaces](surfaces.md). Tests, CI jobs, release guards and the measurements still to take are in [testing and release](testing-and-release.md). Everything here targets `main` at `52917f33e` (v1.94.1).

## How to read this document

Load bearing claims carry a label:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, builds or audio |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main` |
| [R] | Reported by a third party's source code, documentation or issue tracker (upstream whisper.cpp and the `cmake` crate included), not measured here |
| [I] | Inferred; the text names the measurement that settles it when it matters |

Identifier schemes, all indexed in the plan:

| Scheme | Meaning |
|---|---|
| R1 to R8 | The owner's requirements |
| S1 to S20 | The canonical surfaces |
| DD-1 onward | Design decisions in the plan's decision register |
| PR-01 to PR-33, A-1 | Pull requests in delivery order (PR-07 and PR-32 withdrawn), and the owner's one setup action |
| M-1 onward | Measurements still to take (M-9 and M-13 withdrawn) ([testing and release](testing-and-release.md)) |
| O-1 onward | The owner's decisions of 2026-11-05 and the plan's defaults |
| MSG-… | Exact text a person sees ([surfaces](surfaces.md), "Message catalog") |
| P-1 to P-16 | Privacy invariants, each held by a named test |
| Build rules 1 to 11 | The build rules of DD-10 (never called R1 to R11, which are the owner's requirements) |

This document elaborates DD-2 to DD-32, DD-87, DD-89 to DD-91 and DD-93. It restates an answer from the register only where an implementer needs it in place; the reasons and the rejected alternatives stay in the register. Where this document and the register disagree, the register wins. Code is cited as a file plus a symbol; a line number appears only where the plan verified it at `main`.

Privacy tiers, here as everywhere in Biorouter, are a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary. Every refusal in this document is accident prevention.

## The engine and its pin

### Why whisper.cpp

Biorouter vendors whisper.cpp and drives it as a library through a C shim of about ten functions, loading whisper.cpp's legacy ggml `.bin` models, with no bindgen (DD-2). Three measured facts decided it:

1. It had the highest recall of every engine tried once the dictionary prompt went in as BPE token ids. With the 14 term prompt as ids, `whisper-large-v3-turbo-q5_0` reached 93 % and `whisper-small.en-q5_1` 90 % pooled strict recall on the synthetic benchmark set, against 80 % and 85 % with the same prompt as text [M]. On real speech the same change raised turbo from 86 to 94 % and `small.en` (f16) from 76 to 87 % pooled strict [M].
2. Both measured accuracy levers, the token prompt (`prompt_tokens`) and the decoder bonus (a `logits_filter_callback`), are fields of `whisper_full_params`. Only a program that fills that struct reaches them [M].
3. A static whisper.cpp linked into a Rust binary built and ran in Biorouter's own cross images on every shipped target ([What the build spike measured](#what-the-build-spike-measured)) [M].

It is MIT licensed and ships the Silero voice detector as a ggml file [R upstream `models/download-vad-model.sh`, `models/for-tests-silero-v6.2.0-ggml.bin`]. When `whisper_full_params.vad` is set, `whisper_full` runs voice detection itself, opening the detector from the path string in `vad_model_path` [R upstream `include/whisper.h` `vad`, `vad_model_path`; `src/whisper.cpp` `whisper_full`, `whisper_vad`].

### What was rejected

| Candidate | Why not |
|---|---|
| `whisper-server` (upstream's HTTP server) | Takes the prompt as text only; answers every origin with `Access-Control-Allow-Origin: *`, has no key, and its `POST /load` takes any path [R] |
| `whisper-rs` 0.16 | Vendors whisper.cpp 1.8.3, below the CVE-2026-110810 floor; its bindgen step needs libclang, which neither cross image has [C `scripts/cross-env.sh`] |
| transcribe.cpp | A 0.x API with two migration guides already; its Whisper scored the same once whisper.cpp got BPE ids [M] |
| `sherpa-onnx` with Parakeet | Fastest on CPU (123 ms against 730 ms per utterance on 4 threads [M]), but a second native stack of about 33 MB, English only, and its build script downloads prebuilt archives with no checksum [R] |
| `Qwen3-ASR` on a second `llama-server` | `llama-server` is absent from the CLI packages and the Docker image [C `packaging/biorouter-cli.yaml`, `Dockerfile`]; the 0.6B model wrote the wrong gene symbol [M]; the 1.7B Q8_0 file is 2,165,034,944 B, over GitHub's 2 GiB asset limit [M] |
| Candle | No prompt support and no measured accuracy |
| whisper.wasm in the page, the Web Speech API | Rejected by DD-1: a second engine, or recognition that may run off the machine |

### The pin

The engine is pinned to whisper.cpp commit `4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad` on master (2026-11-03; it reports `1.9.5-dev`, with ggml `0.28.0` [M]). Every measurement in this design used that commit. The `v1.9.5` tag is `d1be6fde11ac6e0407606b4e42fe72d34add8037` (2026-10-06); nothing was measured there (DD-3).

whisper.cpp 1.9.4 and earlier carry CVE-2026-110810, an out of bounds read in `whisper_model_load`. The fix is commit `60c0be6`. The pin is four weeks after the tag, so it contains the fix [I from dates]: the clone used during the design is shallow, so the ancestry check could not run there [M]. PR-08 proves it in a full clone and records the result:

```bash
git clone https://github.com/ggml-org/whisper.cpp && cd whisper.cpp
git merge-base --is-ancestor 60c0be6 4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad && echo contains-fix
```

If the check fails, PR-08 stops. The pin then moves to a commit that contains the fix, and only after the procedure below has run at the new commit.

### Changing the pin

A pin moves only in a pull request that does all of this:

1. Choose a commit on upstream master or a tag. Clone in full (not shallow).
2. For every id in `pin.json` `contains`, run `git merge-base --is-ancestor <id> <new commit>`. Any failure stops the bump.
3. Copy the vendored subset ([Vendoring](#vendoring)) from the new commit, regenerate `source-manifest.json`, and update `pin.json` (`commit`, `ggml` from `ggml/CMakeLists.txt`, `date`).
4. Run the retired name guard (R-31) over the new text.
5. Build in the three images (`rust:1.92-bullseye` cross, `rust:latest` mingw cross, native macOS for both architectures) and in the Docker builder. Bullseye's GCC 10.2 is the risk: a later ggml may need GCC 11 in the static path [R]. The fallback is a builder with a newer GCC and a glibc 2.31 sysroot, never a newer base image (that raises the glibc floor [C `scripts/cross-env.sh` comment at `LINUX_RUST_IMG`]).
6. Run the self test in every package smoke and the Nehalem check of DD-12.
7. Run a benchmark subset and the Tier 2 gate of [testing and release](testing-and-release.md): no strict recall regression over 5 points on the PriMock57 subset, the Fareez et al. 2022 subset or the synthetic dictation set, each taken alone, at fixed model and settings (O-38).
8. Update the whisper.cpp line in `NOTICE`; the `NOTICE` version test fails otherwise.

## Vendoring

### Layout

The engine source is vendored the way `vendor/computer-use/` is: a pristine copy, a patch folder, a pin and a manifest [C `vendor/computer-use/`]. A build reads it with no network.

```text
vendor/whisper.cpp/
  source/                                   pristine upstream files at the pin, only what the build reads
    CMakeLists.txt  cmake/  include/  src/  LICENSE
    ggml/                                   CMakeLists.txt, cmake/, include/, src/
    examples/miniaudio.h                    miniaudio 0.11.24, compiled only by biorouter-dictation-capture
    models/for-tests-ggml-base.en.bin       586,836 B   (fixture)
    models/for-tests-ggml-tiny.bin          575,451 B   (fixture)
    models/for-tests-silero-v6.2.0-ggml.bin 885,098 B   (fixture, byte identical to the production detector)
    samples/jfk.wav                         352,078 B   (fixture)
  patches/                                  empty in v1
  pin.json
  source-manifest.json                      path, size and SHA-256 of every file under source/
  LICENSE                                   whisper.cpp's MIT text
  NOTICE                                    what is vendored, from where, and miniaudio's license choice
  README.md                                 how to change the pin
```

"Only what the build reads" may also leave out ggml backend folders the build never enters. ggml adds a backend's folder only when its `GGML_<BACKEND>` option is on [R upstream `ggml/src/CMakeLists.txt` `ggml_add_backend`], and the build turns on only CPU and Metal. At the pin, keeping `ggml/src/ggml-cpu/` and `ggml/src/ggml-metal/` and dropping the other backend folders leaves 177 files and 7,249,911 B under `ggml/`, against 1,484 files and 23,517,546 B for the whole `ggml/` folder [M]. The whole `source/` tree of that choice without the fixtures (the top level `CMakeLists.txt`, `cmake/`, `include/`, `src/` and `LICENSE`, the trimmed `ggml/`, and `examples/miniaudio.h`) is 216 files and 12,065,668 B [M]. PR-08 chooses; the guard proves either choice, because a build from the copy fails on a missing file.

Patches, if one is ever needed, are applied to the copy in `OUT_DIR` at build time, never to `source/`. v1 has none.

### `pin.json` and `source-manifest.json`

```json
{"upstream": "ggml-org/whisper.cpp", "commit": "4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad", "contains": ["60c0be6"], "ggml": "<version>", "date": "2026-11-03"}
```

`ggml` is filled from `ggml/CMakeLists.txt` at vendoring time; at this commit it is `0.28.0` [M]. `source-manifest.json` lists every file under `source/` with its size and SHA-256. The fixture hashes at the pin are [M]:

| File | SHA-256 |
|---|---|
| `models/for-tests-ggml-base.en.bin` | `1bc042ca584ff1895897e95bffb34ccf357be46c1fca97cf7fbe32f2060aa9e8` |
| `models/for-tests-ggml-tiny.bin` | `c486fb9f14a28b1c1dc252741a431646cc573450c900b9d9c406e10294aa01e6` |
| `models/for-tests-silero-v6.2.0-ggml.bin` | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` (the catalog's `silero-vad-v6.2.0` pin) |
| `samples/jfk.wav` | `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` |

### Guards

| Guard | What it checks | Runs in |
|---|---|---|
| `scripts/check-vendored-whisper-cpp.sh` (new, PR-08) | The manifest both ways: every listed file present with its hash, no unlisted file. Every file is committable (`git check-ignore` matches none; the Copilot helper's vendored tree, `vendor/computer-use/`, hit 15 ignored files on its first attempt [C CLAUDE.md]). `pin.json` has `upstream`, a 40 digit `commit`, `contains: ["60c0be6"]`, `ggml` and `date` | `rust.yml` `guards`, `just check-everything` |
| `scripts/check-dictation-fixtures.sh` (new, PR-08) | At most 2.75 MiB of fixtures in all, 1 MiB per file, 0.75 MiB of audio; the four fixtures total 2,399,463 B | `rust.yml` `guards` |
| The retired name guard (R-31) | Scans the vendored text like every tracked file | `rust.yml` `guards` |
| `build.rs` rule 9 | The vendored tree is unchanged after CMake ran | every build |
| A leaf crate unit test | Hashes the four fixtures, so a CRLF conversion or a Git LFS pointer fails loudly | `cargo test -p biorouter-dictation --lib` |

`.gitattributes` gains `vendor/whisper.cpp/source/** -text`, `*.wav binary` and `*.flac binary`.

The retired name guard found 0 text hits and 0 path hits in the whisper.cpp tree at the pin with the guard's own patterns [M]. The remaining risks are tokenizer text files and the correction word list. `vocab.json`, `tokenizer.json` and the decoded vocabularies of both model families spell the retired name, and `merges.txt` does so for the multilingual (turbo) vocabulary [M]. No tokenizer text file ships (the token prompt is built from the loaded model's own vocabulary, see [dictionary](dictionary.md)); the word list generator removes every word the guard matches.

### Fixtures inside the vendored tree

The four fixtures are upstream's own files, so they sit under `source/` and share the manifest. The `for-tests` models have zero tensors: `whisper_full` with one returns 0 and empty text in about 130 ms, and `whisper_tokenize` on its vocabulary gives production token counts [M]. That is what lets the self test and the offline tests run in CI with no network and no real model. The detector fixture has real weights and equals the production detector byte for byte [M].

### miniaudio

The terminal's capture child compiles miniaudio 0.11.24 from the vendored `examples/miniaudio.h` [R upstream header, `MA_VERSION_*`]. One vendored tree carries both, under one guard. miniaudio is offered under the Unlicense (public domain) or MIT No Attribution [R upstream header]. Only `crates/biorouter-dictation-capture` compiles it, and only `biorouter-cli` depends on that crate (DD-5); the capture child itself is specified in [surfaces](surfaces.md).

## The worker

### Entry point and roles

The engine never runs inside `biorouterd` or an interactive `biorouter` process. It runs in a worker: the running binary started again with a hidden first argument (DD-4). `ggml_abort()` always ends in `abort()` [R upstream `ggml/src/ggml.c`], and more than 2,000 `GGML_ASSERT` and `GGML_ABORT` lines lead there: about 2,450 in the CPU and Metal subset and about 5,500 in the whole `ggml/` tree at the pin [M]. A `x86-64-v3` binary also dies with `SIGILL` on an older processor [M]. In a worker, none of that ends a chat. Nothing is discovered on disk; the precedent is the `__br-sandbox` marker [C `crates/biorouter-sandbox/src/shell_sandbox/linux.rs` `HELPER_MARKER`].

The check sits in `main()` of both binaries, right after `complete_libnotify_init()`, whose own documentation requires it to run first, before any thread [C `crates/biorouter/src/fork_safety.rs`]:

```rust
// crates/biorouter-server/src/main.rs:85; crates/biorouter-cli/src/main.rs:24 has the same first
// two statements in a `main` that returns `ExitCode`
fn main() -> anyhow::Result<()> {
    biorouter::fork_safety::complete_libnotify_init(); // before any thread; see fork_safety
    #[cfg(feature = "local-dictation")]
    biorouter_dictation::worker::run_if_invoked(); // returns only when argv[1] is not the marker
    biorouter::execution::runtime::run_on_agent_stack(|| {
        biorouter::execution::runtime::build_agent_runtime()?.block_on(async_main())
    })?
}
```

So a worker starts before `run_on_agent_stack`, the Tokio runtime, logging, configuration, `run_shell_sandbox_helper_if_invoked()` and `app_folder::move_for_this_process()` [C both `main.rs`]. A worker never writes a log file, never reads `config.yaml` and never moves the Windows legacy folder.

From PR-25, `biorouter`'s `main()` answers the `capture` role itself, directly before that call, through `crates/biorouter-dictation-capture`, a crate only `biorouter-cli` depends on (DD-5). `biorouterd` has no such line, so its `run_if_invoked` sees `capture` and exits 2:

```rust
// crates/biorouter-cli/src/main.rs, from PR-25
#[cfg(feature = "dictation")]
biorouter_dictation_capture::run_if_invoked(); // only the capture role; returns otherwise
#[cfg(feature = "local-dictation")]
biorouter_dictation::worker::run_if_invoked();
```

```rust
// crates/biorouter-dictation/src/worker/mod.rs (engine feature)
pub const WORKER_MARKER: &str = "__br-dictation-worker";
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

pub fn run_if_invoked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_MARKER)) {
        return;
    }
    let role = args.next();
    let rest: Vec<std::ffi::OsString> = args.collect();
    let code = match role.as_ref().and_then(|r| r.to_str()) {
        Some("engine") => on_worker_thread(engine::serve_stdio),
        Some("self-test") => on_worker_thread(move || self_test::run(&rest)),
        // "capture" is answered by biorouter's main() before this call; here it falls through to 2
        _ => 2,
    };
    std::process::exit(code);
}

/// Windows pins the main thread's stack at 1 MiB in the executable header, which is why
/// `run_on_agent_stack` exists; the worker gets its own thread with 8 MiB.
fn on_worker_thread(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    std::thread::Builder::new()
        .name("br-dictation-worker".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(f)
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or(70)
}
```

| Role | Binary | Purpose |
|---|---|---|
| `engine` | `biorouter` and `biorouterd` | The transcription worker, speaking the frame protocol on stdin and stdout |
| `capture` | `biorouter` only | The terminal's microphone child ([surfaces](surfaces.md)), in `crates/biorouter-dictation-capture`; `biorouterd __br-dictation-worker capture` exits 2 (P-10), because capture code is linked only into `biorouter` ([The crate and its features](#the-crate-and-its-features)) |
| `self-test` | both | The offline release smoke ([Self test](#self-test)) |

The marker is not a clap subcommand, so it never appears in `--help` and never collides with `biorouter dictation …`. Exit codes: 0 after stdin end of file or `shutdown`, 2 for a role this binary does not have, 20 for an unsupported processor, 21 when the self test is handed a file that is not pinned, 22 when one of its checks fails, 65 when the engine role reads a frame it cannot parse or one over a cap (it writes nothing more), 70 when the worker thread panicked. The marker, the role names and these codes are in the leaf crate in every build, engine or not, so the supervisor can read them; without a linked engine every role answers 2.

### Frame protocol

Frames go both ways as `[u32 LE json_len][json][u32 LE payload_len][payload]`, with a cap of 32 MiB per frame. The cap fits 600 s of 16 kHz audio (19.2 MB) in one `vad_segments` request. Stdout carries frames only; stderr carries filtered library warnings. Audio payloads are PCM s16le, mono, 16 kHz, half the bytes of f32; the worker converts to f32. Every message has a `type` and the protocol version is 1.

### Messages

| Message | Direction | JSON fields | Payload |
|---|---|---|---|
| `hello` | worker to parent, first frame | `protocol: 1`, `whisper: "4da27c41"`, `ggml`, `backends: ["metal", "cpu"]`, `cpu: {avx2, fma, f16c, bmi2, dotprod}`, `isa`, `system_info` (the string `whisper_print_system_info()` returns; `""` when the worker's own CPU gate refused, because calling into ggml on such a processor could itself fault, and the worker then exits 20) | none |
| `load` | parent to worker | `model_path`, `model_sha256`, `model_size`, `vad_dir`, `vad_file`, `use_gpu`, `n_threads`, `warm_up` (whether the Metal encoder warm up follows the load; true when absent, false only from a process that loads one model for one recording, such as `biorouter transcribe`) | none |
| `loaded` | worker to parent | `load_ms` (the model and both detectors), `warmup_ms` (the warm up that follows them, reported apart), `backend: "metal" \| "cpu"`, `device`, `rss_bytes` (peak resident size), `n_vocab`, `multilingual` | none |
| `set_bias` | parent to worker | `revision`, `scope` (the key of one filtered set: `"public"`, `"all"`, or `"private:"` followed by the target affiliation's key, because private rows are filtered by institution, O-32 [I]), `entries: [{written, bias: "auto" \| "always"}]`, `params` (the decoder bonus parameters of [dictionary](dictionary.md)) | none |
| `bias_ready` | worker to parent | `revision`, `scope`, `nodes`, `build_ms` | none |
| `transcribe` | parent to worker | `id`, `bias: {revision, scope} \| null`, `prompt_terms` (ordered, best last), `prompt_budget_tokens: 150`, `language`, `decode` (DD-47), `vad` (DD-48) | PCM, at most 25 s |
| `result` | worker to parent | `id`, `text` (timestamps stripped), `segments: [{t0_ms, t1_ms, text}]`, `prompt_terms_used` (how many terms from the tail made the token budget), `timings: {vad_ms, encode_ms, decode_ms}` | none |
| `vad_segments` | parent to worker | `id`, `vad` | PCM, at most 600 s |
| `segments` | worker to parent | `id`, `spans: [{t0_ms, t1_ms}]` | none |
| `cancel` | parent to worker | `id` | none |
| `shutdown` | parent to worker | none | none |
| `error` | worker to parent | `id` or null, `code: "model_load_failed" \| "bad_audio" \| "cancelled" \| "internal"`, `message` (a fixed English phrase, never text, prompt or audio) | none |

The supervisor refuses a worker whose `hello.protocol` is not 1 and names the binary in the error (`worker <program> speaks protocol 2; this build speaks 1`); that matters for a `BIOROUTER_DICTATION_WORKER` override. It reads `protocol` before any other field, because another version's fields may differ. Frames are capped at 4 MiB of JSON and 32 MiB in all; the header alone decides, so an oversized frame is refused before anything is allocated. Unknown fields are ignored and an unknown `type` is answered `error {code: "internal", message: "unsupported message"}`. `set_bias` is always answered `bias_ready`, never `error`: a worker with no model loaded keeps nothing and reports `nodes: 0`, and a `transcribe` that names a table the worker does not hold decodes without the bonus. That keeps a failed table build from failing the decode the supervisor may already have queued behind it. `params` is a `BonusParams` object whose missing fields take DD-46's frozen values. The worker keeps `BIAS_TABLES` (4) tables by (scope, revision), least recently used first out, and clears them at every `load`, because a table is built over one model's vocabulary; `revision` changes with the store's in process generation and with the rows the table holds, so a mixing policy that changed while the store did not still gets a fresh table. A second `load` of the model already loaded (same hash, paths and backend) is answered from memory. The codec is `crates/biorouter-dictation/src/protocol.rs`; the worker's loop is `worker/serve.rs`.

### Inside the worker

The worker splits its work across two threads: a reader thread that parses stdin frames, and the engine thread that owns the one `whisper_context` and writes every stdout frame.

| Step | What the worker does |
|---|---|
| Load | Opens `model_path` with `std::fs::File` and checks its length equals `model_size`. It feeds the bytes through a `whisper_model_loader` to `whisper_init_with_params` [R upstream `whisper.h`], never through `whisper_init_from_file_with_params`, because upstream converts a UTF-8 path to UTF-16 only under `_MSC_VER` [R upstream `src/whisper.cpp`], so the shipped mingw build would fail on a profile path such as `C:\Users\José\…` |
| Voice detector | The integrated detector inside `whisper_full` takes a path string (`vad_model_path`) and opens it the same narrow way [R upstream `src/whisper.cpp` `whisper_vad_init_from_file_with_params`, which converts to a wide path only under `_MSC_VER`]. The worker sets its own current directory to `vad_dir` (Rust opens directories by wide path on Windows) and passes the bare ASCII file name `vad_file`. A separate detector context for `vad_segments` loads through a `whisper_model_loader` with `whisper_vad_init_with_params` |
| Vocabulary | Right after the load, reads the bytes of every id below end of text once (`Engine::vocabulary`), so neither the first prompt nor the first bonus table pays for it |
| Prompt | Encodes `prompt_terms` with canonical byte level BPE over the loaded model's vocabulary, joins them with ", ", drops terms from the head until the prompt fits 150 tokens (`recognize::prompt::fit`), passes the ids as `prompt_tokens`, and answers how many terms it kept as `prompt_terms_used`. The last prompt encoded is kept, so a stream's later segments, the tail decoded at stop among them, reuse its ids. The prompt itself costs decode time: on small.en with Metal, 14 terms (62 tokens) added about 7 ms to a 5 s clip in a release build, and 30 terms (about 115 to 150 tokens) about 20 ms in a debug build [M, 2026-11-06]. The rules are in [dictionary](dictionary.md); the code is `engine/bpe.rs` |
| Decoder bonus | Builds the byte trie of `set_bias` and installs it as `logits_filter_callback` for each decode that names it. Tables are kept by (scope, revision) in an LRU of 4 and dropped at every load, so the model is part of the key. The callback is reentrant, allocates nothing, never touches an id at or above end of text, and runs inside `std::panic::catch_unwind`. The code is `engine/booster.rs` |
| Decode | `whisper_full` on the context's default state, one request at a time, with the DD-47 parameters (below) and at most 25 s of audio per call |
| Cancel | An `abort_callback` polls an atomic flag set by `cancel` or by stdin end of file. Cancel latency is one encoder pass or one decode step [R] |

The decode parameters the shim sets (DD-47; the reasons are in the register):

| Field | Value |
|---|---|
| strategy | `WHISPER_SAMPLING_GREEDY`, `best_of = 5` |
| `language` | `en` for a `.en` model; the configured language for turbo, never `auto` unless set |
| `translate`, `no_context`, `no_timestamps`, `single_segment` | false, true, false (timestamps stripped from the text), false |
| temperature | 0, with `temperature_inc 0.2` (kept by M-22, 2026-11-06) |
| thresholds | `entropy_thold 2.4`, `logprob_thold -1.0`, `no_speech_thold 0.6` |
| suppression | `suppress_blank = true`, `suppress_nst = false` |
| prompt | `prompt_tokens` set; `initial_prompt` unset |
| context params | `flash_attn` on; `use_gpu` from `load` |
| printing | `print_realtime`, `print_progress`, `print_timestamps`, `print_special` all false (DD-13) |

Voice detection uses threshold 0.50, minimum speech 250 ms, minimum silence 100 ms and `speech_pad_ms = 30` (DD-48). Both sets of values landed with PR-09, because its self test runs the detector and a decode: `engine::decode_params` and `engine::vad_params` in the worker, and `protocol::DecodeOptions::dd47` and `VadOptions::dd48` on the parent's side, which every `transcribe` frame carries and a test holds equal to the worker's. PR-12 added the parameter snapshot with `biorouter transcribe`, the first command that decodes real speech: `the_decode_parameters_snapshot` in `worker/serve.rs` follows the frame through the worker's conversion, every field of `DecodeParams` and `VadParams`, and through the shim's one parameter builder the fields its `br_params_view` reads back (the strategy, `best_of`, `no_context`, `no_timestamps`, `single_segment`, the print flags, the detector switch, `temperature_inc`, `suppress_nst` and the thread count). The thresholds, `suppress_blank`, `translate`, the temperature, the language, the prompt fields and `flash_attn` are checked on `DecodeParams` only, until that view reads them back too.

What runs where, so every host shares one pipeline:

| Worker (C and C++ behind the shim) | Parent (Rust, leaf crate and core service) |
|---|---|
| Model load, the integrated detector, `vad_segments`, BPE encoding of the prompt, the decoder bonus callback, `whisper_full`, abort | PCM validation and resampling, the 25 s split, the bias set (core `bias.rs`), ranking and prompt term order, the nonspeech filter and echo check, correction, the change list |

### The shim

`shim/br_whisper.{h,cpp}` owns `whisper_full_params`, which is passed by value, so Rust needs no struct layout from `whisper.h`. Rust declares the functions by hand in `src/engine/ffi.rs` (build rule 8). A sketch:

```text
// crates/biorouter-dictation/shim/br_whisper.h
typedef struct br_reader { void *ctx; size_t (*read)(void *, void *, size_t); bool (*eof)(void *); void (*close)(void *); } br_reader;
typedef struct br_decode { const char *language; int n_threads; float temperature_inc; /* every DD-47 field */ } br_decode;
typedef struct br_vad { const char *model_file; float threshold; int min_speech_ms; int min_silence_ms; int speech_pad_ms; } br_vad;
typedef void (*br_logits_cb)(const int32_t *tokens, int n_tokens, float *logits, int n_vocab, void *user);

struct whisper_context *br_init(const br_reader *model, bool use_gpu, bool flash_attn);
void  br_free(struct whisper_context *ctx);
int   br_full(struct whisper_context *ctx, const br_decode *decode, const br_vad *vad,
              const float *pcm, int n_samples, const int32_t *prompt, int n_prompt,
              br_logits_cb cb, void *cb_user, const volatile int32_t *abort_flag);
int   br_n_segments(struct whisper_context *ctx);
const char *br_segment_text(struct whisper_context *ctx, int i);
void  br_segment_span(struct whisper_context *ctx, int i, int64_t *t0_ms, int64_t *t1_ms);
int   br_token_bytes(struct whisper_context *ctx, int32_t id, char *buf, int cap);  /* vocabulary for bpe.rs */
int   br_tokenize(struct whisper_context *ctx, const char *text, int32_t *tokens, int cap); /* whisper_tokenize, tests only (PR-14) */
int32_t br_token_eot(struct whisper_context *ctx);
void *br_vad_init(const br_reader *model, int n_threads);
int   br_vad_spans(void *vad, const float *pcm, int n_samples, const br_vad *params, int64_t *spans, int cap);
void  br_vad_free(void *vad);
void  br_log_install(void);   /* whisper_log_set and ggml_log_set with the DD-13 filter */
const char *br_system_info(void);
```

### Supervisor and service API

The leaf crate's `supervisor.rs` spawns, frames, times and restarts the worker. It has no Biorouter dependency, so the command it runs is a value handed in by the core crate (DD-4: "the worker command is a value"). The core crate's `crates/biorouter/src/dictation/service.rs` owns one supervisor per host process in a `OnceLock`, like `llamacpp_sidecar::global` [C `crates/biorouter/src/providers/llamacpp_sidecar.rs:369`].

```rust
// crates/biorouter/src/dictation/service.rs, as PR-10 built it
pub fn service() -> &'static DictationService; // one per process: biorouterd, an interactive biorouter

impl DictationService {
    pub fn status(&self) -> EngineStatus;                                  // never spawns, never waits
    pub async fn prepare_with(&self, spec: LoadSpec) -> Result<Loaded, DictationError>; // spawn and load, on arm
    pub async fn transcribe_with(&self, spec: LoadSpec, stream: u64, pcm: PcmSlice, req: TranscribeRequest)
        -> Result<TranscribeResult, DictationError>;                       // at most 25 s
    pub async fn vad_segments_with(&self, spec: LoadSpec, pcm: PcmSlice) -> Result<Vec<Span>, DictationError>; // at most 600 s
    pub async fn probe_worker(&self) -> Result<WorkerProbe, DictationError>; // status --verbose
    pub async fn threads(&self) -> u32;                                    // DD-32
    pub async fn unload(&self);                                            // before a delete
    // PR-11: catalog ids in place of paths
    pub async fn resolve(&self, id: Option<&str>) -> Result<LoadSpec, DictationError>; // lookup order, verification cache
    pub async fn prepare(&self, id: Option<&str>) -> Result<Loaded, DictationError>;   // None: the configured model
    pub async fn warm_up(&self, id: &str) -> Result<WarmUp, DictationError>;          // one decode after an install
    pub async fn bench(&self, id: Option<&str>) -> Result<(String, BenchRecord), DictationError>;
    // PR-12: a whole recording
    pub async fn resolve_for(&self, id: Option<&str>, language: Option<&str>)
        -> Result<LoadSpec, DictationError>;                               // checks the language too
    pub fn spawn_ahead(&self) -> Result<(), DictationError>;               // the process starts now
    pub async fn transcribe_recording_with(&self, spec: LoadSpec, pcm: Arc<[i16]>, req: TranscribeRequest)
        -> Result<DictatedRecording, DictationError>;                      // file.rs, then correction (PR-14)
    // PR-14: the decoder bonus table is built in the worker when a dictation starts;
    // since M-22 (PR-14b) only when measurement turns the bonus on, so by default it sends nothing
    pub fn install_bias(&self, dictionary: &DictionaryRequest) -> Result<(), DictationError>;
}

pub struct TranscribeRequest {
    pub language: String,
    pub cancel: CancellationToken,
    pub dictionary: Option<Arc<DictionaryRequest>>, // PR-14: bias_set's value, ranked, with its corrector
    pub decode: Option<DecodeOptions>,              // measurements only (M-22); None is DD-47
}

pub struct DictatedRecording {      // PR-14
    pub transcript: RecordingTranscript, // text corrected, raw_text as decoded, segments corrected
    pub changes: Vec<Change>,            // offsets in characters of text
    pub bias_scope: BiasScope,
    pub correct_us: u64,
}

pub struct EngineStatus {
    pub compiled: bool,
    pub state: EngineState,           // no_engine | unsupported_cpu | stopped | starting | ready | error
    pub backend: Option<String>,      // "metal" | "cpu"
    pub device: Option<String>,
    pub threads: u32,
    pub missing_cpu_features: Vec<String>,
    pub rss_bytes: Option<u64>,
    pub build: Option<EngineBuild>,   // whisper, ggml, isa; None without an engine
    #[serde(skip)] pub cpu_sentences: Vec<String>, // MSG-CPU or MSG-CPU-ROSETTA, then MSG-CPU-VM
}

pub enum DictationError {       // Display: the catalog's sentences, MSG-DIAG-EXIT to MSG-DIAG-CANCELLED for the worker
    NotCompiled, Policy(Allowance), UnsupportedCpu { sentence: String },
    WorkerExited { code: Option<i32> }, Deadline { phase: &'static str }, Protocol(String),
    ModelLoadFailed { reason: String }, BadAudio, Cancelled, Internal(String),
    UnknownModel(String), NotInstalled { id: String }, NoModel { host: String, account: String },
    EnglishOnly { model: String },   // PR-11; EnglishOnly is MSG-LANGUAGE-EN
    LanguageUnsupported { language: String }, // PR-12: neither `auto` nor a code whisper.cpp knows
}   // ServerMemory (PR-15) arrives with its catalog text
```

`LoadSpec` (model path, SHA-256 and size, detector path, `use_gpu`, `n_threads`, where 0 asks the service for DD-32's count) and `PcmSlice` (a stretch of a shared `Arc<[i16]>` recording, so a job is written to the pipe without a copy) come from the leaf crate. PR-11 adds the catalog over these: `resolve` turns a catalog id, or the configured `BIOROUTER_DICTATION_MODEL` when none is named, into a `LoadSpec` through the lookup order and the verification cache, and `prepare` loads it, and PR-15 maps `TranscribeResult` to the HTTP shape of [Request and response shapes](#request-and-response-shapes). The detail the engine doc's first sketch lacked is kept, not dropped: `Deadline` names its phase, `ModelLoadFailed` and `Internal` carry the worker's fixed phrase, and `Protocol` names the worker binary. The leaf crate is a dependency of the core crate in every build, with whisper.cpp only behind `local-dictation`, so a build without the feature has the same API: `status()` answers `compiled: false, state: no_engine`, and every other call answers `NotCompiled` (MSG-NO-ENGINE) before anything is spawned. Every call that could spawn passes the same gates first: an engine is linked, the CPU gate does not refuse (MSG-CPU and friends), and `BIOROUTER_DICTATION_WORKER`, when set, is absolute.

The leaf crate's supervisor (`supervisor.rs`) is one actor task per service. Callers send it commands and await one-shot answers; a writer task writes frames, a reader task reads them and a third keeps stderr, so no caller ever blocks on the pipe and a stuck worker cannot stall a deadline. Its tests run every deadline and rule on tokio's paused clock against an in-memory worker; a real process cannot share a paused clock, because tokio advances paused time whenever the runtime is idle, including while it waits on a pipe.

`probe_worker` refuses with the policy's sentence (MSG-POLICY), before anything starts, while a managed `enabled: false` holds; `biorouter dictation status --verbose` checks the same thing first. The other calls leave the policy to their doors (PR-12's `transcribe`, PR-15's routes, PR-25's terminal), because reading the managed file on every request would put file reads on the path from stop to text.

### Lifecycle and deadlines

| Event | Behavior (DD-7) |
|---|---|
| The daemon starts | `AppState::new` starts one background warm up of the active model 5 s later (`spawn_startup_warm_up`, PR-15): spawn, load with the encoder warm up, and one decode of 2 s of the bench clip, because decoder kernels compile on their first use too; then the normal idle unload. It runs only on Metal, outside an administrator's account, with a model installed and the policy allowing dictation, and only when this executable (path, size and modification time) differs from the one recorded at the last warm up in `dictation/warm-up.json` in the local data folder, which a successful warm up writes. It never delays startup or a request: a stream that arrives meanwhile shares the worker and its load |
| Rescan places a speech model, or the person chooses another one (`POST /dictation/models/select`) | The daemon runs the same warm up of that model in the background, as after an install, with the active model last so it is the one left loaded (PR-15): the startup warm up covers only the model that was active when the daemon started, and a load whose pipelines the Metal cache lacks costs 3 to 4 s. Metal only, never in an administrator's account, and skipped while a stream is open, because the worker holds one model and a dictation in progress would otherwise load its own again at the stop (an install's warm up follows the same rule). Measured live with both models already compiled, the warm up after a choice took 182 ms to a ready `small.en` and 404 ms to a ready turbo, then one decode of 75 and 255 ms [M, 2026-11-06] |
| The person arms a microphone, or a file transcription starts | `prepare_with` spawns the worker first (`spawn_ahead`), works out the thread count while the process starts, and sends `load` once `hello` arrives; status reports `starting` until `loaded`, so loading overlaps speech. The arming door does not await it. A dictation stream does this when it is created (PR-15) |
| No request for `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS` (default 300) | The supervisor sends `shutdown`, waits 5 s, then kills; process exit returns all CPU and GPU memory |
| `hello` not received | Killed after 60 s |
| `loaded` not received | Killed after 180 s |
| `result` not received | Killed after max(30 s, 10 × audio seconds) on the GPU, max(120 s, 20 × audio seconds) on the CPU. Only a `transcribe` on a Metal worker is a GPU request: `vad_segments` runs on the separate detector, which the shim opens with `use_gpu = false`, so it gets the CPU deadline on any worker |
| The worker exits or is killed during a GPU request | The same request is retried once in a fresh worker with `use_gpu = false`; the supervisor stays on the CPU until its next idle unload |
| The worker exits, is killed or answers `model_load_failed` during a GPU `load` | The Metal warm up runs inside `load`, so this is where a GPU that cannot serve shows. The same `load` is sent once more, in a fresh worker, with `use_gpu = false`; the waiting requests stay queued for it. If it loads, the supervisor stays on the CPU until its next idle unload; if it fails too, the GPU was not the cause (a damaged file fails both ways), the requests get the error, and the next load asks for the GPU again |
| Any other worker exit | The request fails with a typed error; the next request spawns a fresh worker; the audio stays with the caller for a retry |
| The parent dies | The worker reads end of file on stdin, sets the abort flag and exits; no pid file is needed |
| The person presses Ctrl-C in a terminal | On Unix the worker leads a process group of its own (`process_group(0)`), so the SIGINT a terminal sends to its foreground group reaches the interactive `biorouter`, which cancels a turn, and never a warm worker. On Windows the worker has no console (`CREATE_NO_WINDOW`) |
| The worker crashes | No memory reaches the disk (P-1): the engine role sets its core file limit to zero before it reads a frame, on Linux also clears its dumpable flag, and on Windows turns off Windows Error Reporting for its own crashes (`SetErrorMode` with `SEM_NOGPFAULTERRORBOX`) and asks that a report still made, as for a fail fast abort, leave out the heap (`WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP)`) |
| A model is deleted or replaced | `unload()` runs first, because on Windows a file opened without `FILE_SHARE_DELETE` (as MSVC `fopen` opens it) cannot be deleted while it is open; whisper.cpp closes the model file after loading, so the conflict exists only during a load |

Measured on Apple Silicon with Metal [M], on a machine under heavy load, so the absolute times are upper bounds:

| Measure | Value |
|---|---|
| First worker with a cold Metal cache (the first dictation after a model install, after an app update that changes ggml's kernel sources, or after the shared per user Metal cache evicted the entry) | 3.0 to 4.2 s to ready, of which 2.8 to 3.9 s is the shader compile of the embedded library |
| The same compile on GitHub's macos-latest runner (macOS 26, arm64, a Metal cache that starts empty), debug build, three workers starting at once | about 46 s from spawn to a refused `load`, nearly all of it before `hello`, because `hello` reads ggml's backend registry and that creates the Metal device; a `--version` of the same build run first had finished in under 0.3 s, and workers started after those three said `hello` in about 0.1 s (rust.yml run 42256072334, 2026-11-06). That is 14 s short of the 60 s `hello` deadline, so `tests/dictation_worker.rs` runs one worker to its `hello` before any test that times the worker or starts several at once |
| First exec of the installed 155 MB `biorouterd` after a reboot | about 1.3 s, then 28 to 121 ms |
| Warm spawn to ready, before the engine had its encoder warm up: `base.en`, `small.en`, turbo `q5_0` | 146 to 169 ms, 209 to 216 ms, 264 to 279 ms |
| Warm model load inside a benchmark process | 0.12 to 0.36 s |
| `Engine::load` as the first load of a fresh process (Metal device, model and warm up, no process start), `small.en` f16 and turbo `q5_0`, warm file and Metal caches | 373 to 436 ms and 578 to 755 ms, of which the encoder warm up is 59 to 63 ms and 258 to 263 ms; with the warm up removed the first request was 6 to 14 ms slower than the steady state (2026-11-05, M4 Max, load average 15 to 60) |
| Idle footprint of a warm worker: `small.en`, turbo `q5_0` | about 690 MB, about 730 MB |

Through the real supervisor and worker (`worker_latency_real_models` in `crates/biorouter-cli/tests/dictation_worker.rs`, a debug parent driving the Release engine; 2026-11-06, M4 Max, macOS 26.7.1, load average 10 to 25 with other agents' builds running beside it, so the CPU rows are upper bounds; the Metal rows matched within a few milliseconds across two runs) [M]:

| Measure (36 clips of 3 to 6 s, three voices) | turbo `q5_0` | `small.en` f16 | `base.en` f16 |
|---|---|---|---|
| Spawn to `hello` | 77 ms | 79 ms | 77 ms |
| `hello` to `loaded`, Metal (`load_ms` + `warmup_ms`) | 351 ms (98 + 253) | 175 ms (102 + 72) | 74 ms (46 + 27) |
| Spawn to ready, Metal, first worker of the run for turbo | 428 ms | 254 ms | 151 ms |
| First request after `loaded`, Metal | 279 ms | 103 ms | 47 ms |
| Steady state per utterance, Metal: median, p90 | 283, 294 ms | 110, 130 ms | 53, 61 ms |
| Warm decode on Metal of a 2 s, 5 s and 11 s clip (median of 5) | 261, 285, 316 ms | 77, 110, 153 ms | 34, 56, 88 ms |
| Steady state, 8 CPU threads: median, p90 | 4.5, 6.0 s | 884 ms, 1.16 s | 212, 232 ms |
| Warm decode on 8 CPU threads of a 2 s, 5 s and 11 s clip | 4.8, 4.7, 4.5 s | 683, 717, 769 ms | 189, 207, 238 ms |
| Steady state, 4 CPU threads: median, p90 | not run | 1.69, 2.29 s | 366, 375 ms |
| `vad_segments` on 60 s (one detector thread) | 107 ms | 109 ms | 110 ms |
| Re-arm after `unload` (spawn to ready again), Metal, warm file cache | 443 ms | 225 ms | 153 ms |
| Peak resident size at `loaded`, Metal; peak footprint after the row | 823 MB; 964 MB | 778 MB; 917 MB | 355 MB; 489 MB |
| 60 s dictation streamed in real time, stop to text | 309 to 330 ms, Metal | 729 ms, 8 CPU threads (4.2 s in a run at load average 25) | |

Where a warm decode goes, from the worker's own timings: turbo's encoder takes 242 ms on Metal (4.4 to 4.8 s on 8 CPU threads) and `small.en`'s 650 ms on 8 CPU threads, whatever the length, because the encoder always runs Whisper's full 30 s window; the decoder grows with the words (turbo on Metal 13, 29 and 45 ms for 2, 5 and 11 s); and everything before the first encoder pass, the integrated detector and the log mel spectrogram, costs about 2.5 ms per second of audio (6.5, 13.5 and 27.6 ms). The supervisor, the pipe and the parent add 1 to 4 ms. So a short utterance on turbo pays mostly its fixed encoder pass; sizing `audio_ctx` to the segment (L-9, M-10) is the lever that would change that.

In the streamed rows the replay cut 10 segments while it played, each a sentence or two of 4 to 10 s (at 10.0 s buffered the open second sentence kept the cut to the first), and at stop the last sentence, 5.9 s with its pads, was still undecoded: the stop cost the tail's detection (11 ms) and one decode (282 to 293 ms on turbo, 702 ms on 8 CPU threads). In the slow CPU run a segment cut 1.4 s before stop was still decoding, so the tail's detection waited 1.47 s behind it and the final decode took 2.68 s on a loaded machine. The first request after `loaded` was within 7 ms of the steady median on Metal for every model, so the warm up in `load` leaves nothing for the first dictation to pay on a warm Metal cache. The self test took 347 to 364 ms on the CPU and 209 to 220 ms on Metal, in both binaries.

Through the `/dictation/*` routes (PR-15: `stop_to_text_on_the_catalog_models`, ignored, in `crates/biorouter-server/tests/dictation_routes.rs`; the routes in process, a debug daemon driving the Release engine on Metal, a warm worker, 12 bench clips of 3.5 to 6.0 s replayed in real time with a `PUT` every 2 s and the rest carried by `finish`; 2026-11-06, M4 Max, load average 10 to 22 with other builds running) [M]: stop to text median 121 ms (108 to 187) on `small.en` q8_0 and 299 ms (287 to 317) on turbo `q5_0`, of which the tail's decode was a median 115 ms and 292 ms, so the routes, correction and the transcript add about 6 ms. `timings.wait_ms` was 0 or 1 ms in every run: the model loaded while the clip played. The first dictation into a stopped worker, created with the stream, still answered 109 ms (`small.en`) and 290 ms (turbo) after its stop, because the load (and a debug build's first hash of the model file) fell inside the clip. A second run, with the segmenter's two progress rules, gave medians of 135 ms and 302 ms at a load average of 10 to 20 (one `small.en` clip at 473 ms while another build ran). A 60 s dictation of the clips end to end, replayed in real time: with a 400 ms pause between sentences, 11 segments were decoded, ten of them while it played, and the stop cost a 3.3 s tail: 117 ms on `small.en` and 378 ms on turbo from stop to text. With no pause at all, the forced 25 s cuts left a 10 s tail: 347 ms and 457 ms, against 1,142 ms and 1,316 ms before the rule, with the same text length (811 and 984 characters), so the cuts lost no words there. Both left one segment to decode at stop, so the 8 s trigger (`STREAM_VAD_TRIGGER_MS`) stays; M-19 still measures boundary word loss on real conversation.

The real dev `biorouterd` in a sandbox, driven over loopback HTTP as the stream client sends (5 bench clips in real time, turbo on Metal, the same day, load average 14 to 22) [M]: stop to text median 303 ms (292 to 363), about 8 ms above the tail's decode. Its startup warm up ran once for the new executable and finished 10 s after the start (the 5 s delay, then a debug build's first hash of the 574 MB model, the load and one decode: ready 549 ms, decode 258 ms), and the next start of the same executable ran none.

PR-15's live check, the same evening [M, 2026-11-06]: the dev daemon started from clones of one build at four paths, in a sandbox whose models Rescan placed, with `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS` at 15 so that a cold case meets a stopped worker, driven over loopback HTTP with a `PUT` every 2 s and the tail on `finish`, at a load average of 10 to 47 with other agents' builds beside it. The clips were `s11_Karen` (5.1 s) and an 11 s one (10.7 s: `s11_Karen`, 0.5 s of silence and `s12_Daniel`), whose first sentence was decoded while the second was spoken. Stop to text in ms, two cold and three warm runs per cell, across two runs of the matrix (the second with the 14 benchmark terms in the dictionary):

| | turbo `q5_0`, 5 s | turbo, 11 s | `small.en` q8_0, 5 s | `small.en`, 11 s |
|---|---|---|---|---|
| Warm worker | 296 to 311 | 304 to 338 | 124 to 150 (and 348 and 460 at a load average of 47) | 118 to 130 |
| Worker stopped at creation | 300 to 496 | 307 to 484 | 127 to 204 | 129 to 142 |

`timings.wait_ms` was 0 in every run, so the load started at creation always finished while the clip played; a 1.5 s utterance into a stopped worker answered 97 ms (`small.en`) and 276 ms (turbo) after its stop, again with no wait. Some turbo decodes took 40 to 190 ms longer than the usual 290 to 300 ms, on a warm worker as often as on one stopped at creation: seven pairs of a dictation into a stopped worker and one right after it gave tail decodes of 293 to 437 ms and 292 to 438 ms, so a worker stopped at creation costs nothing the next dictation does not. With the dictionary the 11 s clip came out word for word (`scRNA-seq`, `PHD2`). A 10 minute stream uploaded at once, as a replay sends it, answered 14.2 s (turbo) and 10.1 s (`small.en`) after `finish`: a `finish` that follows the upload at once leaves the whole recording to the tail, about 30 decodes. A 5 s stream started 1 s into it answered 582 ms after its stop, against 494 ms alone, one long segment ahead of its tail. Every one of the 123 clips of the ten minutes appeared in the transcript once no sentence followed itself; the same sentence said by three voices back to back, inside one packed segment of the replay, was written once by both models, so a test of word loss must not repeat a sentence. The ninth stream answered 429 with `Retry-After: 5`; a resent `seq` with other bytes answered the first chunk's `received_s`, and the transcript held the first bytes; a `seq` ahead answered 409 with `expected_seq`; a stream untouched for 61 s answered 410 to a `PUT` and to a read, whether or not its transcript had been read; install, select and delete without the proof answered 403 `proofRequired` with MSG-INSTALL-AGENT, and every route answered 401 without the secret. A worker killed with SIGKILL during a segment's decode was replaced and its job retried, so the stream still answered, 1,171 ms after its stop, and the next dictation took 503 ms on `small.en`, on the CPU until the idle unload. The startup warm up ran once per new executable path: 3,459 ms to a ready turbo model the first time, then 805 ms from a second path holding the same bytes; and a rebuilt daemon's first load (the warm up after Rescan) took 509 ms, so a new executable did not by itself meet a cold Metal cache here. No transcript phrase, stream id or audio file was left in a log or in the sandbox.

A worker killed in the middle of a real turbo transcription (`a_worker_killed_mid_transcription_real_model`, ignored, same run) [M]: during a Metal job the same job was retried in a new worker on the CPU and answered 4.65 s after the kill; during the following CPU job the request failed with `WorkerExited` 8 ms after the kill, and the next request ran in a new worker. An `unload` then put the supervisor back on Metal. `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS: 2` in `config.yaml` stopped the service's worker 2.01 s after it was ready (`the_idle_unload_follows_config_yaml`, ignored).

The worker still answers `loaded` after its warm up, not before it as the M1 design's latency finding 15.4 item 1 asked. That item is deferred: splitting the warm up out of `Engine::load` changes PR-09's engine, and the measurement above shows no first request cost to recover on a warm Metal cache (M1-L7). On a cold Metal cache the 3 to 4 s of pipeline compiles fall inside `load` either way, and a request that arrives meanwhile waits for them in both designs; what the split would change is that `status` reads `starting` until the warm up ends, and that a GPU fault in the warm up is a load fault (which the CPU load retry above covers) rather than a fault in `Ready`.

A cold `transcribe_with` (nothing loaded, no `prepare_with` first) through the CLI's own service, against the `for-tests` fixture on the CPU, four rounds each (`cold_transcribe_latency` in `dictation_worker.rs`; 2026-11-06, M4 Max, with other builds running) [M]: with the thread count worked out first and the request sent after it, as PR-10 first did, median 483 ms and p90 729 ms; with the worker started before the sidecar is asked, median 410 ms and p90 473 ms. The sidecar probe alone took a median of 132 ms and at most 305 ms, past its 250 ms bound, because `llamacpp_sidecar::status()` builds a new HTTP client, which loads the system's root certificates, before its first await, and a timeout around a future cannot cut short work done inside one poll. The probe now runs on a task of its own, so the bound holds (a second measurement, three rounds: median 141 ms, at most 224 ms, against 233 ms inline the same hour) and the caller's task never runs that work. `prepare_with` and `transcribe_with` both overlap it with the process start (about 80 ms to `hello`), so a cold start still waits about 50 ms for it (median 382 ms with the worker started first, 484 ms with the probe first); a probe that reuses one client would remove that.

Any utterance longer than about 0.5 s hides a warm start of `small.en`, and about 0.8 s one of turbo with its encoder warm up. A cold start is hidden by a 5 s utterance; after a 2 s utterance the person waits 1 to 2 s more, and after a one word command about 3 s [M]. The `arming` phase of the composer covers the rest ([surfaces](surfaces.md)).

### Concurrency

One worker per host process, one `whisper_context`, one request at a time (DD-6). The integrated detector runs only through `whisper_full` (and `whisper_full_parallel`), which call `whisper_vad` on the context's default state; `whisper_full_with_state` never runs it [R upstream `src/whisper.cpp`]. So v1 has no second decoder state. Requests from several chats, windows and streams queue first in, first out by stream, served round robin one segment (25 s or less) at a time, so a long dictation never starves a short one. `vad_segments` requests go before any transcription, because they gate the segmentation of a dictation still in progress. A job for another model than the loaded one waits for the running job, then the supervisor sends `load` again. The supervisor reports its queue (`streams`, `segments`) for PR-15's 429 rule and does not cap it itself. Several decoder states wait for M-14.

The segment planner is `audio::plan_closed_segments` and `audio::plan_final_segments` (DD-50). While the person speaks it plans nothing until 8 s lie past the decoded mark; then every span that ended more than 0.3 s before the buffered end becomes one segment, padded by 0.3 s that never reach past the middle of the pause before the next span, and split only to respect the 25 s cap, at the longest pause or every 25 s inside speech with no pause. At stop, everything after the mark is planned. PR-12 and PR-15 call it; PR-10 tests it and the latency measurement below uses it.

A whole recording goes through the same planner (PR-12, `crates/biorouter/src/dictation/file.rs`, behind `DictationService::transcribe_recording_with` and `biorouter transcribe`). The detector runs over windows of at most 600 s, the most one `vad_segments` takes; each window's closed spans are planned at once, and every segment is queued for decoding before the next window is detected, so the worker decodes while the detector reads on, and the supervisor runs detector jobs before queued decodes. A window with no speech moves the mark to one second before its end; speech that never pauses is cut 25 s before the window's end and detected again from there, so no segment passes 25 s and every span is decoded once. The planned segments are then packed: consecutive ones are joined while the result stays within 25 s (`file::pack`), so every cut that remains is one the planner made at a pause. The planner splits at the longest pause level by level, and where pauses are alike that peels off one sentence per level: a 13.6 minute recording of sentences 2 s apart took 60 decodes unpacked and 40 packed, and turbo on Metal went from 21.4 s to 16.8 s (the table below), because each decode pays a whole encoder pass [M]. Exact zeros cost no request at all, and a recording in which the detector finds nothing costs one detector request and no decode. One model load serves the whole recording. Each decode's pieces that are only bracketed or parenthesized tags are dropped, the echo, recital and collapse checks of [dictionary](dictionary.md) run against the prompt that was sent, and the joined text goes through the whole output filter (`recognize/text.rs`, `recognize/echo.rs`). A retry drops only the prompt and keeps the decoder bonus, so with no prompt sent (an empty dictionary, `--no-dictionary`) no retry is made: it would decode the same audio the same way. From PR-14 the service corrects the joined text once, after every decode of the recording has returned, so the changes refer to the final text, and corrects each segment on its own for the subtitle formats.

`biorouter transcribe` orders its work for latency: after the refusals that need nothing (no engine, the processor, the managed switch, the language code), it starts the worker process, then decodes the file on a blocking thread while the model's files are verified and loaded. A recording under half a second, or of exact zeros, is answered before the model is needed. The load does not wait for DD-32's thread count, which asks the llama sidecar and took about 100 ms [M], longer than finding a model in the account's folder and starting the worker; only the load's warm up reads the load's count, and every decode carries DD-32's. The dictionary is prepared beside the load. The worker decodes the English word list, which only a bonus table reads, on a thread of its own at every model load; that kept the first bonus table's build the only dictionary work between `loaded` and the first detector request, but since the bonus is off by default the decode (about 20 ms of CPU in a debug build [M]) is wasted on every load, `--no-dictionary` included, and competes with the load on a machine with few cores. PR-15, which owns `worker/serve.rs`, decodes it only when a bonus table is coming (a flag on the `load` frame, or the first `set_bias`). With the 14 term benchmark dictionary, a debug build's paired runs (84 per model, on the 42 synthetic clips, each run with and without `--no-dictionary`) spent a median 16.5 ms more with the dictionary on `small.en` and 15.5 ms on turbo, 11 to 14 ms of it in the decode (the prompt's tokens and the unoptimized bonus callback) and about 1.6 ms correcting [M, 2026-11-06]. Since PR-14b the bonus is off by default (M-22), so no table is built or sent and nothing runs between `loaded` and the first detector request; measured the same way after it, with the prompt ending in its period, the dictionary cost a median 9 ms of `total_ms` on `small.en` (7 ms in the decode) and 7 ms on turbo (5 ms), against 6 and 7 ms more when `BIOROUTER_DICTATION_MEASURE_BONUS=1` turns the bonus back on; `total_ms` medians 277 against 287 ms (`small.en`) and 492.5 against 502 ms (turbo); preparing the dictionary took a median 21 ms (at most 31) beside a model load of 164 to 205 ms; `--no-dictionary` prepares nothing (`dictionary_ms` 0) [M, 2026-11-06, debug CLI, M4 Max shared with other work]. After the text is written the process raises the `uses` of the terms it holds and exits. As PR-14b first built it (the store's lock waited for up to 50 ms, and `usage.json` was flushed to disk twice, each an `F_FULLFSYNC` on macOS, while standard output stayed open until the exit) that added a median 12 ms between the text and the exit (32 against 44 ms) [M], and a reader of a pipe (`biorouter transcribe a.wav | pbcopy`, `$(...)`, a script capturing the output) got the end of the text only at the exit. Since the review fix the lock is tried once, the file is replaced by a rename with no flush to disk (an empty file left by a power cut starts over), and on Unix standard output is pointed at `/dev/null` right after the text, so a pipe's reader sees the end of the text before the usage write starts. Measured on 36 paired runs per arm (`small.en`, Metal, debug CLI, the 14 term dictionary, the M4 Max shared with other work), the usage write added a median 15.7 ms (p90 29.2 ms) to both the end of the output and the exit before, and after it nothing measurable: a median of -1.0 ms to the end of the output and -0.1 ms (p90 8.0 ms) to the exit, against the same binary with `--no-dictionary` [M, 2026-11-06]. Only a caller that waits for the exit, such as `-o out.txt && …`, still waits for that write. PR-14b's live check measured that write on 336 paired runs (both models, the 42 synthetic clips, two dictionaries holding the 14 terms under different ids, debug CLI, catalog models, the M4 Max shared with other work): the exit came a median 1.0 ms later after the end of the output with the dictionary than with `--no-dictionary` (3.3 to 3.5 ms against 2.2 to 2.4 ms, later in 296 of the 336 pairs), and with the store's lock held by another process the batch was dropped and the exit came 3.2 to 4.7 ms after the text. Process start to the end of the output, medians per clip: `small.en` 321 and 385 ms with `--no-dictionary` against 333 and 390 ms with the dictionary (paired, a median 8 to 10 ms more), turbo 557 and 549 ms against 557 and 552 ms (paired, -1 to 4 ms); the two figures are the two dictionaries' runs [M, 2026-11-06]. All 336 dictionary runs (84 per model, with the bonus and without) ended their sentence with `.`, `?` or `!`, where before the tail PR-14's live check found turbo dropping it on 14 to 26 of 42 clips [M].

`biorouter transcribe` measured end to end, a release build on an M4 Max with Metal, models in the account's folder, each run a fresh process as a script would start it (2026-11-06) [M]:

| Measure | `small.en` q8_0 | turbo `q5_0` |
|---|---|---|
| 42 bench clips of 3.5 to 6 s: process start to exit, median (max) | 279 ms (299) | 497 ms (515) |
| of which the model ready, from the command's start (worker start, load, the detector's warm up) | 136 ms | 175 ms |
| of which the recording, once the model is ready (detector and decode) | 110 ms | 291 ms |
| An 84 s recording, 14 sentences: WAV, and the same as a 44.1 kHz stereo MP3 | 1.42 s, 1.40 s | 2.43 s, 2.42 s |
| A 13.6 minute recording (two detector windows), 126 sentences, 40 decodes | 10.4 s | 16.8 s |
| Exact zeros (no worker request) | 21 ms | 30 ms |
| A noise clip (one detector request, no decode) | 225 ms | 463 ms |

The MP3's decode and resampling (105 ms for 84 s) runs while the model loads, so it adds nothing. A long lived worker's `load` ends with an encoder warm up on Metal (one encoder pass and one decoder step: 56 ms for `small.en`, 250 ms for turbo [M]), so the first dictation finds the encoder ready. `transcribe` decodes one recording and exits, so it skips that pass: its `load` frame carries `warm_up: false` (`DictationService::one_shot`, `Engine::load_with_warm_up`), and only the detector's own warm up (2 ms) runs. Measured on the same 42 clips, run clip by clip in turn with the release build from before the change (2026-11-06, M4 Max, warm Metal cache) [M]: the command's own total fell from 294 to 246 ms median for `small.en` and from 713 to 467 ms for turbo, and the model was ready after 136 ms instead of 186 ms and 175 ms instead of 425 ms, while the recording's own time moved by 3 ms (107 to 110 ms, 288 to 291 ms) and every transcript was the same. The table's first three rows are the build with the change; its rows for the 84 s and 13.6 minute recordings and the noise clip were measured before it and include the warm up once. The daemon and the terminal front ends keep the warm up. A model in a read only folder (the environment folder, the system folder, a managed folder) is hashed again by every process (DD-17), so each `transcribe` from such a folder pays that hash before the load: 137 to 179 ms for `small.en` and 253 to 357 ms for turbo in the release build [M]. A record of verified read only copies that outlives the process would remove it; that is a change to DD-17's verification, left for later.

The desktop daemon and an interactive `biorouter` on the same account each own a supervisor, so two copies of a model can be resident at once when both dictate. The idle unload bounds how long, and status reports `rss_bytes` for each.

### Environment

```rust
// crates/biorouter/src/dictation/service.rs
pub fn worker_command(program: &Path) -> tokio::process::Command { // program = worker_program(), below
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg(biorouter_dictation::worker::WORKER_MARKER).arg(biorouter_dictation::worker::ENGINE_ROLE)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);                               // a terminal's Ctrl-C never reaches a warm worker
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("GGML_") || key.starts_with("WHISPER_") { // GGML_VK_* included
            cmd.env_remove(key.as_ref());
        }
    }
    cmd.env("GGML_NO_BACKTRACE", "1");                  // no debugger fork on abort
    if biorouter_dictation::cpu::rosetta_translated() { // sysctl.proc_translated == 1
        cmd.env("ROSETTA_ADVERTISE_AVX", "1");
    }
    crate::subprocess::prepare_agent_child_command(&mut cmd); // last: strips daemon secrets, hides the console
    cmd
}
```

`prepare_agent_child_command` must run after every other `env` call, because the strip and `env` write the same map [C `crates/biorouter/src/subprocess.rs:54`]. `worker_program()` is:

1. `BIOROUTER_DICTATION_WORKER` when set: an absolute path, refused otherwise; refused again when its `hello.protocol` is not 1.
2. On Linux, `/proc/self/exe`, so a package upgrade during a session still starts the build that is running [I].
3. Elsewhere, `std::env::current_exe()`. A symbolic link to the CLI and a lone Windows `.exe` copy both work [M].

The worker opens no socket, writes no file and reads no configuration (P-12).

### Log hygiene

The worker calls `br_log_install()` before anything else. Its callback, installed with both `whisper_log_set` and `ggml_log_set`, forwards only warnings and errors to stderr and drops any line over 200 bytes (DD-13). Upstream logs the model path at info level, so the path never reaches stderr. The supervisor keeps the last 64 stderr lines in a ring buffer. It shows them, with control characters escaped, only in `biorouter dictation status --verbose` and the diagnostics bundle, never in a log file, a notification or a session row (DD-93, P-12).

### Self test

```bash
biorouter __br-dictation-worker self-test \
  --model vendor/whisper.cpp/source/models/for-tests-ggml-base.en.bin \
  --vad   vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin \
  --wav   vendor/whisper.cpp/source/samples/jfk.wav
```

The same command runs under `biorouterd`. In order, it:

1. Runs the CPU gate; on an unsupported processor it prints MSG-CPU and exits 20.
2. Hashes `--model` and `--vad` and refuses a file whose SHA-256 is neither the pinned `for-tests` hash nor a catalog hash (exit 21), so a release binary carries no general "load any file" path (P-11).
3. On x86, reads `whisper_print_system_info()` and fails (exit 22) unless it reports AVX2, FMA, F16C and BMI2 as compiled in [I for the exact feature names], which catches a build that silently fell back to SSE2 (build rule 2).
4. Loads the model through the same loader as the engine role.
5. Runs voice detection: at least one segment in `jfk.wav`, none in 5 s of generated silence (exit 22 otherwise).
6. Runs `whisper_full` on `jfk.wav`: return code 0 and empty text, because the model has zero tensors [M].
7. Prints one JSON line and exits 0:

```json
{"selftest":"ok","whisper":"4da27c41","ggml":"<version>","isa":"x86-64-v3","backend":"cpu","elapsed_ms":130}
```

On Apple Silicon `isa` is `armv8.2-a+dotprod+fp16` and `backend` is `metal`, or `cpu` on a host where Metal does not initialize, such as a virtual machine without a GPU [I]. The self test runs in every package and release smoke (DD-12).

**Acceptance (PR-09, PR-10).** `tests/dictation_engine_offline.rs`, the self test unit tests, the supervisor tests with a fake worker and `crates/biorouter-cli/tests/dictation_worker.rs` pass as listed in [testing and release](testing-and-release.md); `worker_entry_runs_first` finds the call right after `complete_libnotify_init()` in both `main()` functions; `biorouter dictation status --format json` validates against schema 1 in both CLI package smoke containers.

## Native build and platform floors

### The crate and its features

```toml
# crates/biorouter-dictation/Cargo.toml
[package]
name = "biorouter-dictation"
version.workspace = true          # scripts/check-version-consistency.sh fails a hardcoded version
edition.workspace = true

[features]
default = []
engine = ["dep:cmake", "dep:cc"]  # whisper.cpp, the shim, the worker; the only feature

[build-dependencies]
cmake = { version = "0.1.58", optional = true }   # already in Cargo.lock
cc = { version = "1.2.64", optional = true }      # already in Cargo.lock

[dependencies]
reqwest = { version = "0.12.9", default-features = false, features = ["rustls-tls-native-roots", "stream"] }
sha2 = "0.10"
fs2 = "0.4.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { workspace = true }
```

The leaf crate has no Biorouter dependency and takes every path as an argument (DD-5). `reqwest` stays on rustls, which `scripts/check-no-native-tls.sh` enforces [C]. Feature wiring in the other crates:

| Crate | Feature | Contents |
|---|---|---|
| `crates/biorouter` | `local-dictation` | `["biorouter-dictation/engine"]`. The leaf crate itself is a plain dependency from PR-10, not an optional one, so `dictation::service` has one API in every build; without `engine` the leaf crate compiles only the protocol, the supervisor, the planner and the CPU gate, with no CMake. PR-09 had it optional |
| `crates/biorouter-server` | `default = ["jemalloc", "local-dictation"]` | `local-dictation = ["biorouter/local-dictation", "dep:biorouter-dictation"]` (the direct dependency is for `run_if_invoked`) |
| `crates/biorouter-cli` | `default = ["jemalloc", "local-dictation", "dictation"]` | `local-dictation` as in the server (PR-09); `dictation` = `local-dictation` + `symphonia` 0.5.5 with only `wav`, `pcm`, `aiff`, `flac` and `mp3` (PR-12) + `dep:biorouter-dictation-capture` (PR-25); without it `transcribe` answers MSG-NO-ENGINE. DD-78's size rule dropped AAC and Vorbis: with them and their MP4 and Ogg readers symphonia added 1,719,376 B to the stripped release `biorouter` on macOS arm64, and 1,696,984 B to a stripped x86_64 test binary; without them the `dictation` feature (these readers and the `transcribe` code) adds 556,496 B to that `biorouter`, with and without it built from one tree, and symphonia alone adds 590,896 B to a stripped probe binary on macOS arm64 [M]. On Linux x86_64, the rule's own platform, a stripped release probe built in the release image (`rust:1.92-bullseye`) grew by 1,766,248 B with them, 1,531,800 B with AAC and MP4 alone, and 650,512 B without them [M]. M4A, AAC, Ogg, Opus and WebM are refused with an `ffmpeg` line |
| `crates/biorouter-dictation-capture` (PR-25) | none | `cc` builds `shim/br_capture.c` over the vendored `examples/miniaudio.h`; depends on `biorouter-dictation` for the frame protocol; `biorouter-cli` is its only dependent |

`biorouter-crew` never links the engine; `cargo tree -p biorouter-crew` must list no `biorouter-dictation` [C `crates/biorouter-crew/Cargo.toml` has no `biorouter` dependency].

Capture is its own crate rather than a feature of this one (DD-5), because a feature on a shared crate cannot keep capture out of `biorouterd`. Every release build compiles both binaries in one Cargo invocation: `cargo build --release --bin biorouterd --bin biorouter` in `scripts/release.sh` `cmd_linux-backend` and `cmd_backends`, and a plain `cargo build --release` in `build_mac_backends` and the Justfile [C]. With `resolver = "2"`, Cargo would compile a shared crate once with the union of the features its dependents ask for, so a `capture` feature would be linked into `biorouterd` too [I from Cargo's feature unification; not built], and `cargo test -p biorouter-server` would not see it, because it leaves the CLI out of the build graph. So `crates/biorouter-cli/src/main.rs` dispatches the `capture` role into `crates/biorouter-dictation-capture` before it calls `run_if_invoked`, which answers 2 for `capture` in every binary; `cargo tree -p biorouter-server -e normal,build` must list no `biorouter-dictation-capture`; and the P-10 checks run on the binaries the release command itself builds (`cross-build-nightly` and `release.sh verify`), as [testing and release](testing-and-release.md) lists.

### Build rules

Each rule of DD-10 was found by a failure in the build spike [M], except rule 7, which was read in the `cmake` crate's source [R].

| Rule | What `build.rs` does | The failure it prevents |
|---|---|---|
| 1 | Copies `vendor/whisper.cpp/source/` into `OUT_DIR/whisper-src` and configures a two line wrapper, `crates/biorouter-dictation/cmake/CMakeLists.txt`, that `add_subdirectory`s the copy | As the top level project, whisper.cpp writes `bindings/javascript/package.json` into its own source tree [R upstream `CMakeLists.txt`]; against a read only mount the build failed, and against the vendored tree it would break the manifest. The vendored subset has no `bindings/`, so the copy must not be the top level project: under the wrapper upstream sets `WHISPER_STANDALONE OFF` and skips that step |
| 2 | `GGML_NATIVE=OFF` always, with `GGML_SSE42`, `GGML_AVX`, `GGML_AVX2`, `GGML_FMA`, `GGML_F16C`, `GGML_BMI2` on for x86; `GGML_CPU_ARM_ARCH=armv8.2-a+dotprod+fp16` for macOS arm64; `armv8.2-a+dotprod` for Linux arm64 | A native build probes the build machine (the arm64 release Mac, the Docker builder); a cross build silently falls back to SSE2 without explicit flags |
| 3 | `BUILD_SHARED_LIBS=OFF`, `GGML_OPENMP=OFF`, `WHISPER_BUILD_TESTS=OFF`, `WHISPER_BUILD_EXAMPLES=OFF`, `WHISPER_BUILD_SERVER=OFF`, `WHISPER_SDL2=OFF`, `WHISPER_CURL=OFF`, `GGML_CCACHE=OFF` | OpenMP adds `libgomp.so.1` or a mingw OpenMP DLL; shared libraries need packaging and discovery |
| 4 | Static libstdc++ on Linux and mingw: resolves `$CXX -print-file-name=libstdc++.a`, then `rustc-link-search` and `rustc-link-lib=static=stdc++` | A dynamic libstdc++ adds `libstdc++.so.6` and `GLIBCXX_3.4.26` and fails `scripts/check-linux-runtime-deps.sh` |
| 5 | mingw: `cargo:rustc-link-lib=static:+verbatim=ggml.a` (and `ggml-cpu.a`, `ggml-base.a`) plus `dylib=advapi32` | ggml drops the `lib` prefix on WIN32, so `-lggml` failed; `ggml-cpu` reads the CPU name from the registry (`__imp_RegOpenKeyExA` undefined) |
| 6 | macOS arm64: `GGML_METAL=ON`, `GGML_METAL_EMBED_LIBRARY=ON`; macOS x86_64: `GGML_METAL=OFF`; both `GGML_BLAS=OFF`; links `Metal`, `MetalKit`, `Foundation`, `Accelerate` and `c++` | An embedded library leaves no `.metallib` to sign; with BLAS on, ggml binds `cblas_sgemm$NEWLAPACK$ILP64`, a macOS 13.3 symbol that is not guarded |
| 7 | `cmake::Config::profile("Release")` whatever the Cargo profile | `cmake` 0.1.58 maps `opt-level` 0 to `Debug` and `s` or `z` to `MinSizeRel` [R `cmake` crate `get_profile`]; the Dockerfile sets `CARGO_PROFILE_RELEASE_OPT_LEVEL=z` [C], and `just run-dev` builds a debug daemon, so both would get an unoptimized engine |
| 8 | No bindgen: the shim is compiled with `cc` and Rust declares its functions by hand | bindgen needs libclang, which neither cross image has |
| 9 | Hashes the vendored tree after CMake and fails the build on any change | The rule 1 trap, or a future upstream step that writes into its source |
| 10 | (Dockerfile) installs `cmake` in the builder stage | `rc=101 … is 'cmake' not installed?` in `rust:1.92-bookworm` |
| 11 | (hermit) pins `cmake` under `bin/` | `bin/` has no `cmake` today [C]; developers used whatever Homebrew had (4.4.5 on the design machine) |

`build.rs` also panics on an Apple target when `MACOSX_DEPLOYMENT_TARGET` is unset, with a sentence that names `.cargo/config.toml` (DD-11).

```text
# crates/biorouter-dictation/cmake/CMakeLists.txt
cmake_minimum_required(VERSION 3.14)
project(br_whisper C CXX)
add_subdirectory(${BR_WHISPER_SRC} whisper)
```

### The MSVC test leg

The `rust.yml` `test` job runs on `windows-latest` with MSVC, while Biorouter ships mingw [C `rust.yml`]. PR-09 either builds the engine under MSVC or compiles a `target_env = "msvc"` stub that answers `no_engine`, and `CLAUDE.md` says which (M-3). Building it is preferred: a stub leaves native Windows execution of the engine untested on every pull request. `windows-latest` has CMake 4.4.5 [R], and `aws-lc-sys` 0.30.0, which already builds through the `cmake` crate [C `Cargo.lock`], runs that build with MSVC in the same job [R CI logs read by the design]. The extra time was estimated at 1.5 to 3 minutes per run on a job measured at 27.8 minutes [I].

Under `target_env = "msvc"`, `build.rs` changes four things:

1. It skips rules 4 and 5 (no libstdc++, no verbatim archive names); the spike's script gated those on `target.contains("windows")` and would fail there.
2. It passes `/utf-8`, because 12 lines of upstream `src/whisper.cpp` carry characters outside ASCII [M].
3. It keeps `GGML_NATIVE=OFF`, `GGML_AVX2=ON`, `GGML_BMI2=ON` and `GGML_OPENMP=OFF` (OpenMP would pull `vcomp140.dll`) and the dynamic CRT `/MD`, which `cc` picks by default.
4. It defines `CMAKE_C_FLAGS_RELEASE` and `CMAKE_CXX_FLAGS_RELEASE` as `/MD /O2 /Ob2 /DNDEBUG`, because with the Visual Studio generator the `cmake` crate sets those variables from `cc`'s flags with every `/O` flag removed [R `cmake` 0.1.58 source].

The model loader of [Inside the worker](#inside-the-worker) removes the one behavioral difference between the MSVC engine and the shipped mingw engine (UTF-16 paths under `_MSC_VER` only).

### Platform floors

| Platform | Floor | How it is held |
|---|---|---|
| Linux x86_64 (GUI deb and rpm, CLI deb and rpm) | `DT_NEEDED` within `BASE_LIBS` (`ld-linux-x86-64.so.2 libc.so.6 libdl.so.2 libgcc_s.so.1 libm.so.6 libpthread.so.0 libresolv.so.2 librt.so.1 libutil.so.1`) [C `scripts/check-linux-runtime-deps.sh`]; highest symbol `GLIBC_2.30` under `GLIBC_MAX="2.31"` [C `scripts/release.sh`, M]; no `GLIBCXX` symbol version [M] | `assert_glibc_floor` and `scripts/check-glibc-floor.sh` already read `biorouter`, `biorouterd` and `biorouter-crew`, so they cover the worker unchanged [C]. `check-linux-runtime-deps.sh` keeps `BASE_LIBS` unchanged and gains a named refusal: `libstdc++.so.6`, `libgomp.so.1`, `libasound.so.2`, `libpulse.so.0` or `libjack.so.0` fails with "link it statically or dlopen it" instead of the advice to add a `PACKAGE_MAP` row |
| Windows x86_64 (zip, Squirrel) | System DLL imports only; none of the three DLLs `WIN_DLL_STAGE` stages (`libstdc++-6.dll`, `libgcc_s_seh-1.dll`, `libwinpthread-1.dll`) [C `scripts/cross-env.sh`, M] | A `cross-build-nightly` step lists `DLL Name` entries with `objdump -p` and fails on those three, `libgomp-1.dll` or `vcomp140.dll` |
| macOS, both architectures | Minimum OS 12.0, the app's own `LSMinimumSystemVersion` [M] | New `.cargo/config.toml` (below, PR-02); `build.rs` panics without the variable; `scripts/check-macos-deployment.sh` in `release.sh` `build_mac_backends` and `cmd_verify` and in the macOS leg of `rust.yml`; a leaf crate test reads its own unstripped test executable |
| x86 processors | `x86-64-v3` (AVX2, FMA, F16C, BMI2) | Build rule 2; the CPUID gate ([CPU and GPU backends](#cpu-and-gpu-backends)); the Nehalem check of DD-12 |

```toml
# .cargo/config.toml (new, PR-02)
[env]
MACOSX_DEPLOYMENT_TARGET = "12.0"
```

With the variable set, rustc stamps `minos 12.0` and `cc` compiles C and C++ with `-mmacosx-version-min=12.0`, on both architectures [M]. Unset, `cc` 1.2.64 compiles at the SDK version (27.2 on the design machine) while rustc stamps 11.0, and the engine linked binary strongly imports `_OBJC_CLASS_$_MTLResidencySetDescriptor`, a macOS 15 class [M]. dyld would then refuse to start the whole `biorouterd` and `biorouter` on macOS 12 to 14 [I from dyld semantics], which no worker can isolate, because it is the same binary. Nothing sets the variable at `main` [C], and the shipped `biorouterd` is `minos 11.0, sdk 27.0` [M]. The bundled `llama-server` is `minos 13.3` [M], so local chat models already need 13.3 while the app claims 12.0; dictation does not.

`scripts/check-macos-deployment.sh <binary>…` reads the minimum with `vtool -show-build`, or from `LC_BUILD_VERSION` or `LC_VERSION_MIN_MACOSX`, and requires 12.0. `xcrun dyld_info` refuses binaries built with `strip = true` [M], so the strong import check runs on the unstripped test executable inside the leaf crate's `--lib` tests, which also asserts that `MTLResidencySetDescriptor` is a weak import.

### The Docker builder

At `main` the builder is `rust:1.92-bookworm` with no `cmake`, it builds only `--package biorouter-cli`, it sets `CARGO_PROFILE_RELEASE_OPT_LEVEL=z`, LTO and one codegen unit, and the runtime stage is `debian:bookworm-slim` [C `Dockerfile`]. PR-09 adds `cmake` to the builder (rule 10). Rule 7 matters most here, because of `opt-level = "z"`; rule 2 matters too, because a Docker build is a native build on whatever machine runs it. With Debian's `cmake` 3.25.1 the engine built in 15 s and transcribed in `debian:bookworm-slim` on arm64, with `DT_NEEDED` of `libgcc_s`, `libm` and `libc` [M]. Without `GGML_CPU_ARM_ARCH`, Linux arm64 gets the NEON only baseline [M], so rule 2 sets `armv8.2-a+dotprod` and the CPU gate checks `dotprod`. PR-31 later builds `biorouter-server` and the web bundle into the image ([surfaces](surfaces.md)).

### The `cross_windows` toolchain

The mingw cross image is `rust:latest` [C `scripts/cross-env.sh` `WIN_RUST_IMG`], and the spike built the mingw worker with rustc 1.98.1 outside the repository [M]. That image already builds with `rust-toolchain.toml`'s 1.92 through rustup's automatic install (the `cross-check` windows job log, 2026-11-02), but the automatic install is deprecated (rust-lang/rustup#4836), so PR-09 adds an explicit `rustup toolchain install` to `cross_windows`. wine 8 cannot start binaries from rustc 1.98, wine 10 can [M]; a native Windows run is M-1.

### Load time guards that land with the engine

The engine turns on by default only in the pull request that also lands these guards (DD-12). A static initializer that a future compiler vectorizes would break `--version` on old processors before any other gate runs, and the measurement that `--version` survives was taken on a test binary, not on `biorouterd` [M].

| Guard | Where | Fails when |
|---|---|---|
| `scripts/check-dictation-isa.sh` (new) | `rust.yml` `cross-build-nightly`, in `debian:bullseye` under `qemu-x86_64 -cpu Nehalem`, with a native positive control | `biorouter --version` or `biorouterd --version` does not exit 0; the self test does not exit 20 with MSG-CPU; `biorouter dictation status --format json` does not report `unsupported_cpu` |
| The self test | `scripts/build-cli-linux-packages.sh` (`debian:bookworm-slim`, `rockylinux:9`), `.github/workflows/linux-gui-packages.yml`, `.github/workflows/windows-gui-packages.yml`, the native Windows publish gate `.github/workflows/release-artifact-smoke.yml`, `scripts/smoke-test-release-artifacts.sh` (`smoke_mac` arm64, and x64 under `arch -x86_64` with `ROSETTA_ADVERTISE_AVX=1` on macOS 15 or later; `smoke_deb`, `smoke_rpm`, `smoke_cli_packages`) | The line `"selftest":"ok"` is missing |
| Linux dependencies | `cross-build-nightly`, `release.sh` | `DT_NEEDED` outside `BASE_LIBS`, a symbol above `GLIBC_MAX` (2.31; the spike measured 2.30), any `GLIBCXX` |
| Windows imports | `cross-build-nightly` | Any non system DLL import |
| Minimum OS | already merged with PR-02 | Anything but 12.0 |
| `scripts/check-dictation-build-prereqs.sh` (new) | `rust.yml` `guards` | The Docker builder has no `cmake`; hermit has no `cmake`; `build.rs` lacks `.profile("Release")`; `biorouter-dictation` reaches `bindgen` in `Cargo.lock` by any route other than `aws-lc-sys`'s own dependency on `bindgen`, which the installer's TLS stack carries and which is optional on every target Biorouter builds (a package below `aws-lc-sys` that needs `bindgen` is refused too). `Cargo.lock` records no features, so the guard does not check that no build turns that optional `bindgen` on; measured on 2026-11-06, `cargo tree --locked -p biorouter-cli -p biorouter-server -e features -i aws-lc-sys --target all` lists only `default` and `prebuilt-nasm`; `cargo tree -p biorouter-crew` shows `biorouter-dictation` |

No pull request job links the Linux or Windows binaries, so every pull request that changes the engine or its build dispatches `rust.yml` before merge.

### Binary growth

Measured per stripped binary [M]:

| Target | Growth per binary |
|---|---|
| Linux x86_64 | +2.17 MB |
| Windows x86_64 (mingw) | +3.84 MB |
| macOS arm64 (Metal embedded) | +6.69 MB |
| macOS x86_64 | +1.69 MB |

The arm64 app carries two engine binaries, so it grows about 13.4 MB. PR-09 records its own figures and expects them within 10 % of these. PR-11 then embeds the 352,078 B bench clip ([Suitability and bench](#suitability-and-bench)), adding about 0.35 MB to each binary on top of PR-09's figures.

## CPU and GPU backends

### Per target

| Rust target | Shipped in | Surfaces | Backend v1 | ISA flags | Hardware class |
|---|---|---|---|---|---|
| `aarch64-apple-darwin` | arm64 dmg and its `Resources/bin` | S1; S9 to S11 on Apple Silicon | Metal, library embedded; CPU after a GPU fault | `armv8.2-a+dotprod+fp16` (every M series chip [I]) | `AS` |
| `x86_64-apple-darwin` | x64 dmg | S2 (and the Intel build under Rosetta) | CPU | `x86-64-v3` | `CPU` |
| `x86_64-pc-windows-gnu` | zip, Squirrel installer | S3, Windows CLI | CPU | `x86-64-v3` | `CPU` |
| `x86_64-unknown-linux-gnu` (bullseye, glibc 2.31) | GUI deb and rpm, CLI deb and rpm | S4 to S8, S10, S19, S20 | CPU | `x86-64-v3` | `CPU` |
| Docker image, amd64 and arm64 (bookworm) | the image (PR-31) | S16 | CPU | `x86-64-v3`; arm64 `armv8.2-a+dotprod` | `CPU` |
| `x86_64-pc-windows-msvc` | CI `test` leg only | none | CPU, or the `no_engine` stub | `x86-64-v3` | none |

No CUDA or Vulkan in v1 (DD-8). Vulkan needs `glslc` and CMake 3.19 or later, and the bullseye image has CMake 3.18.4 and no Vulkan SDK [M].

### Metal

`GGML_METAL_EMBED_LIBRARY=ON` builds the shader source into the binary, so nothing extra is signed, and it needs no Xcode Metal toolchain at build time: the embed step only runs `cat`, `sed` and `echo` and then `.incbin` [R upstream, M on a machine whose toolchain was not installed]. The price is a runtime shader compile whenever the Metal cache is cold: 2.8 to 3.9 s of compile and 3.0 to 4.2 s to ready [M] (the triggers are in [Lifecycle and deadlines](#lifecycle-and-deadlines)). Metal ran under the hardened runtime with `osx-sign`'s default entitlements, with none, and with an empty set, cold compile included; it does not need `allow-jit`, because the compile runs in `MTLCompilerService` [M on ad hoc signed copies]. M-4 (a notarized, hardened runtime launch and one Metal transcription on macOS 12 and 13) gates the first release that carries the engine.

Memory on Metal, from the benchmark processes [M]: turbo peak RSS 0.94 GB (footprint 1.06 GB), `small.en` 0.60 GB. A turbo worker beside a 19 GB Llama Server model on a 32 GB Mac is unmeasured; status reports `backend` and `rss_bytes` so Settings can warn.

### CPU threads

The worker uses `min(8, available_parallelism)` threads, halved (at least 1) while the llama sidecar or a loopback Ollama is serving (DD-32). The service treats the sidecar as serving while `llamacpp_sidecar::global().status()` reports `ready` or `starting` within 250 ms (a slower answer counts as not serving), reuses the count for 10 s so a stream's segments do not each ask again, and works it out while the worker process starts. Ollama counts as serving while a turn in this daemon runs on the `ollama` provider at a private tier, which that provider reports only for a loopback host and a model not forwarded to ollama.com (PR-15: `AppState::local_ollama_serving`, which the daemon installs into the service with `set_local_serving_probe` and the service asks under the same 250 ms bound). A turn is what makes Ollama compete for the processor; a model it merely keeps loaded does not, and the daemon cannot list idle agents bound to it. A process without the daemon's probe (the CLI) counts only the sidecar. `status` never probes the sidecar: it reports the loaded worker's count, or `min(8, available_parallelism)` when none is loaded. Whether `available_parallelism` honors a cgroup v2 CPU quota on a hub node is part of M-14.

### The CPUID gate

The parent checks the processor before it starts a worker, and before any download (DD-9):

```rust
// crates/biorouter-dictation/src/cpu.rs
pub enum HardwareClass { As, Cpu, No }

pub fn detect() -> CpuFeatures;                          // this process's CPUID view; no ggml call
pub fn classify(f: &CpuFeatures, rosetta: bool, macos_major: Option<u32>) -> CpuReport; // pure
pub fn host_report() -> &'static CpuReport;              // OnceLock; never spawns
```

`classify` gives Apple Silicon `As`; Linux arm64 `Cpu` with `dotprod`, else `No`; x86_64 `Cpu` when AVX2, FMA, F16C and BMI2 are all present, else `No` with the list. Under Rosetta, which hides those features from CPUID, an x86_64 build on macOS 15 or later is `Cpu` (the parent starts the worker with `ROSETTA_ADVERTISE_AVX=1`), and on anything earlier `No` with MSG-CPU-ROSETTA. No status call spawns: the earlier sketch's `probe_worker_hello()` inside the gate is replaced by the confirmation the worker gives anyway. The worker runs the gate again on its own CPUID view before it calls into ggml, and refuses when any feature is missing, whatever the parent decided; its `hello.cpu` reports that view, and the supervisor treats a `hello` that lacks a feature, or exit 20, as unsupported for the life of the process and spawns nothing more.

On a host the gate refuses, the engine would die with `SIGILL` (exit 132); the gate turns that into exit 20 and a sentence [M]. The CPU class and the missing features appear in `biorouter dictation status` (and from PR-15 in `GET /dictation/status`) as `engine.state: "unsupported_cpu"` and `missing_cpu_features`, `engine.backend: null`, and the sentence in the text status; PR-11 adds it to `install.reason`.

### Rosetta

Rosetta 2 hides AVX, AVX2, FMA, F16C and BMI2 from CPUID but executes them [M on macOS 26.7.1]. Setting `ROSETTA_ADVERTISE_AVX=1` in a process's environment makes CPUID report them, and the x86_64 worker then transcribes correctly [M]. Rosetta translates AVX2 only from macOS 15 [R]. So when `sysctl.proc_translated` is 1, the parent starts a worker with that variable and reads `hello.cpu`; if AVX2 is still missing (any macOS before 15), it refuses with MSG-CPU-ROSETTA, which tells the person to install the Apple Silicon build.

### Hosts without AVX2

Who loses local dictation under the `x86-64-v3` pin: x86 processors before Haswell (2013) and AMD before Excavator; the Mac Pro (Late 2013), which runs macOS 12 on a processor with AVX but not AVX2 [I]; and virtual machines whose CPU model masks AVX2, which is plausible on lab servers [I]. They are refused before any download, with the routes that still work there:

| Where | Text |
|---|---|
| Desktop | MSG-CPU, then MSG-CPU-DESKTOP (connect the app to a Biorouter server that supports dictation) |
| `serve` and other servers | MSG-CPU-SERVER; when CPUID leaf 1 reports a hypervisor, MSG-CPU-VM follows [I for the detection] |
| The Intel build under Rosetta before macOS 15 | MSG-CPU-ROSETTA |

A static build pinned to SSE4.2 instead would run nearly everywhere, but it was about 3.3 times slower than `x86-64-v3`, measured only under Rosetta translating to NEON, so only the direction and rough size hold [M]. A second helper build for Linux (`GGML_BACKEND_DL` with runtime CPU variants, 17.6 MB against 2.2 MB, and only after dropping two variants that need GCC 11 [M]) is O-11. The owner decides it after M-11, the census of lab hosts:

```bash
grep -o 'avx2\|fma\|f16c\|bmi2' /proc/cpuinfo | sort -u
```

### GPU workers later

`BIOROUTER_DICTATION_WORKER` (env only) names another worker binary that speaks protocol 1. That is how a CUDA or Vulkan worker, or the O-11 helper, would arrive later without callers changing. A desk check of a later Vulkan worker for Windows and Linux [R upstream source and issue trackers] set these rules for it:

1. It is a separate, dynamically linked executable with its own ggml library set, built from whisper.cpp's vendored ggml. `GGML_BACKEND_DL` requires shared ggml [R upstream `ggml/src/CMakeLists.txt`], so it cannot load into the static engine.
2. It never shares a folder or a pin with `llama-server`. `llama-server` b11429 carries ggml 0.26.0 and the pin carries 0.28.0; both declare backend API version 3, so the loader would accept them, but the operation numbering differs, so graphs would compute the wrong operations with no error [R upstream headers].
3. It loads backend modules from an explicit path, never with `ggml_backend_load_all()`, which also searches the current directory [R upstream].
4. It allows only discrete GPUs by default, because on integrated GPUs the decode steps were slower than the CPU in public reports [R], and it falls back to the CPU on any device error.

An off machine transcription endpoint (`BIOROUTER_DICTATION_ENGINE_URL`) is not part of v1 (DD-87, O-12). The managed `external_engine` key is parsed as a no op so a later policy file does not turn v1 off.

## The model catalog and tiers

### Catalog

`STT_MODEL_CATALOG` in `crates/biorouter-dictation/src/catalog.rs` is compiled into the binary and is the only source of model pins (DD-14, DD-17):

| id | file | bytes | SHA-256 | role |
|---|---|---:|---|---|
| `whisper-large-v3-turbo-q5_0` | `ggml-large-v3-turbo-q5_0.bin` | 574,041,195 | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` | default on `AS`; multilingual; 275 ms per 4 to 6 s utterance on Metal, 1.06 GB on Metal, 9.0 to 9.6 s per utterance on the 4 thread CPU proxy [M] |
| `whisper-small.en-q8_0` | `ggml-small.en-q8_0.bin` | 264,477,561 | `67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067` | default on `CPU`, Light on `AS` (DD-14); English only; 127 ms per utterance on Metal, 0.73 to 0.82 s per 5 s utterance on 4 CPU threads of the portable build [M] |
| `silero-vad-v6.2.0` | `ggml-silero-v6.2.0.bin` | 885,098 | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` | required by both; 5.6 to 18.3 ms per clip across clean, noisy and reverberant clips (median 8.5 ms) [M] |

```rust
// crates/biorouter-dictation/src/catalog.rs
pub struct SttCatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: SttKind,                      // Speech | Vad
    pub engine: SttEngine,                  // WhisperCpp
    pub format: SttFormat,                  // GgmlBin (legacy ggml magic, not GGUF)
    pub languages: Languages,               // EnglishOnly | Multilingual | NotApplicable
    pub tokenizer: Tokenizer,               // Gpt2 | Multilingual | NotApplicable
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub urls: &'static [&'static str],      // v1: one GitHub URL each
    pub release_tag: &'static str,
    pub licenses: &'static [LicenseText],   // file name plus include_str! text
    pub requires: &'static [&'static str],  // speech models require "silero-vad-v6.2.0"
    pub classes: &'static [HardwareClass],
    pub default_for: &'static [HardwareClass],
    pub min_total_ram_gib: u32,             // turbo 8, small.en 4, detector 0
    pub accepted_sha256: &'static [&'static str], // earlier revisions that stay valid; contains sha256
}

pub const STT_MODEL_CATALOG: &[SttCatalogEntry] = &[ /* the three pinned entries */ ];
pub const RETIRED_STT_MODELS: &[RetiredSttModel] = &[]; // { id, replaced_by }, the RETIRED_MODELS shape
pub const CATALOG: Catalog = Catalog { entries: STT_MODEL_CATALOG, retired: RETIRED_STT_MODELS };
// Catalog::{entry, resolve, by_sha256, sized, default_for, with_requirements}; the installer
// and the lookup take a Catalog, so their tests run against a synthetic one
```

The pins were compared with what GitHub reports for A-1's immutable releases on 2026-11-06: size and `digest` equal the table for all three, and `gh release verify-asset` passed on the detector [M]. The license texts compiled in from `crates/biorouter-dictation/licenses/` are byte identical to the release assets (SHA-256 `b5d65a59…` for `LICENSE-openai-whisper.txt`, `94f29bbe…` for `LICENSE-whisper.cpp.txt`, `2e63e9a3…` for `LICENSE-silero-vad.txt`) [M].

The shape follows `MODEL_CATALOG` and `RETIRED_MODELS` in `crates/biorouter/src/providers/llamacpp.rs` [C], with one difference that matters: the Llama Server path only compares a file's size and logs "Using it anyway" on a mismatch [C `llamacpp_sidecar.rs` `cached_file_matches`, `warn_if_unexpected_file`], while dictation refuses any file whose SHA-256 differs from a pin. Model ids are what configuration stores (`BIOROUTER_DICTATION_MODEL`) and what every route and command takes; nothing takes a URL.

Catalog invariants, tested in `--lib` (PR-11): every `sha256` is 64 hex digits; every size is under 2,147,483,648 B; every URL starts with `https://github.com/BaranziniLab/biorouter-models/releases/download/stt-`, names its `release_tag` and ends with its `file_name`; no URL contains `huggingface.co`, `api.github.com` or `/latest/`; every `requires` resolves; exactly one speech model is the default for each class; `accepted_sha256` contains `sha256`; every retired id points at a catalog id.

### Measured accuracy and latency

These figures chose the catalog. They are design measurements dated 2026-11-04 and labeled [M], not product claims: under DD-95 no accuracy claim about the product ever appears in user documentation, release notes, the announcement or the interface (the owner's decision O-23, 2026-11-05). The full tables are in [speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md).

| Measure | `whisper-large-v3-turbo-q5_0` | `whisper-small.en-q8_0` |
|---|---|---|
| Synthetic speech, 14 term dictionary prompt as BPE ids, pooled strict recall [M] | 93 % | 90 %, measured on q5_1 (q5_1, q8_0 and f16 agreed within 1 point) |
| The same, prompt as text [M] | 80 % | 85 %, measured on q5_1 |
| Real speech (PriMock57 drug names and MultiMed gene symbols, 162 occurrences; MultiMed is research of 2026-11-04 only, O-38), prompt as BPE ids, timestamps and voice detection on, pooled strict [M] | 94 % | 86 % |
| Real speech, MultiMed gene symbols (research of 2026-11-04 only, O-38), strict, prompt as text [M] | 91 to 92 % | 70 to 75 %, measured on `small.en` f16 |
| Real speech, whole utterances collapsed [M] | none | 1 of 109 with the settings above; in the earlier text prompt study on `small.en` f16, 12 of 49 with timestamps off and 2 of 49 with timestamps on |
| Per 4 to 6 s utterance on Metal, warm [M] | 275 ms | 127 ms (q5_1: 130 ms) |
| Single core encoder work per 30 s window, portable CPU build (no BLAS) [M] | 34.7 s (q8_0: 12.3 s) | 2.46 s (q5_1: 6.95 s) |
| Per utterance on 4 CPU threads (portable ARM build, no BLAS, loaded machine) | 9.0 to 9.6 s per 4 to 6 s utterance, real time factor about 2.1 [M] | 0.73 to 0.82 s per 5 s utterance [M]; q5_1 about 2.1 s [I, projected from its single core work] |
| Peak RSS on Metal; on CPU [M] | 0.94 GB (footprint 1.06 GB); 0.86 GB | 0.7 GB (q5_1: 0.6 GB); 0.585 GB |

Quantization cost no measurable accuracy: on synthetic speech `small.en` q5_1, q8_0 and f16 agreed within 1 point and turbo q5_0 equalled q8_0, and on real speech q5_1, q8_0 and f16 stayed within noise of each other [M]. It does decide CPU speed. On the portable CPU build the q8_0 encoder needs 2.8 times less single core work than q5_0 and q5_1, for `small.en` and turbo alike, while on Metal `small.en` q5_1 and q8_0 run at the same speed (130 ms and 127 ms) [M]. A published x86 matrix multiply benchmark points the same way, more strongly [R].

No x86 machine was used for timing. Scaled from published x86 reports, each class taken as a central multiple of this Mac's single core with a spread of about 50 %, a 5 s utterance on the full window would take [I]:

| x86 class | `whisper-small.en-q5_1` | `whisper-small.en-q8_0` |
|---|---|---|
| Desktop or H series laptop, 8 or more big cores | about 2.7 s | about 1.0 s |
| Typical 4 core U series laptop | about 8.1 s | about 3.0 s |
| Older or low power 4 core | about 20 s | about 7.4 s |

M-10 measures the shipped flags on an Intel Mac, a Windows laptop and a Linux server.

The owner confirmed `whisper-small.en-q8_0` (264,477,561 B) as the CPU default on 2026-11-05 (DD-14); M-10 measures its x86 latency. Its decision rule: after a bench, turbo is `recommended` on a CPU host only when its real time factor is 0.5 or less (before a bench, the rule of [Suitability and bench](#suitability-and-bench) applies); a faster CPU tier (Parakeet or a GPU worker) is opened for a later release if `small.en`'s median exceeds 1.5 s per 5 s utterance on a lab laptop, or its strict recall on Tier 2's real sets (the PriMock57 subset and the Fareez et al. 2022 subset) is below 75 % while turbo's real time factor there exceeds 0.5. Gene symbol recall on real speech is not part of the rule, because no licensed public recording of real speech with gene symbols exists and the owner ruled MultiMed out (O-38).

Why q8_0 and not q5_1 for the `CPU` class (DD-14): on the portable build q5_1 needs 2.8 times the encoder work of q8_0 for no accuracy gain [M], and the projections above put q5_1 at about 8 s per sentence on a typical laptop [I]. M-10 also decides whether `whisper-large-v3-turbo-q8_0`, with the encoder window sized to the utterance and a repetition guard, joins the catalog for hosts with 8 or more big cores: it scored 94 % pooled strict on real speech and took 1.0 s per 5 s utterance on 4 threads here [M], about 0.7 s projected on a fast x86 desktop [I].

### Hardware classes

| Class | Machines | Default speech model | Other speech model |
|---|---|---|---|
| `AS` | Apple Silicon | `whisper-large-v3-turbo-q5_0` | `whisper-small.en-q8_0` (Light) |
| `CPU` | x86_64 with AVX2, FMA, F16C and BMI2 (Intel Mac, Windows, Linux, WSL, servers); Linux arm64 with `dotprod` | `whisper-small.en-q8_0` (DD-14) | `whisper-large-v3-turbo-q5_0` after a warning that names the measured `bench` time; never picked automatically |
| `NO` | Everything the CPUID gate refuses | none; refused before any download | none |

### Suitability and bench

`crates/biorouter-dictation/src/hardware.rs` rates every model for the host (DD-22). It never reuses the llama sidecar's `is_cpu_only()`, which answers false on Windows because `llama-server` there is a Vulkan build [C `llamacpp_sidecar.rs` `is_cpu_only`], while the dictation engine is CPU only on Windows.

| Value | When |
|---|---|
| `unsupported_cpu` | The host class is `NO` |
| `below_minimum` | Total RAM below `min_total_ram_gib`: turbo below 8 GiB, `small.en` below 4 GiB [I thresholds]; installable after a warning |
| `too_slow` | A recorded bench real time factor above 0.5; or turbo on a `CPU` host with no bench record and fewer than 16 physical cores |
| `recommended` | A recorded real time factor of 0.5 or less; or, with no bench record, the class default, or turbo on Apple Silicon or with 16 or more physical cores |
| `suitable` | Any other installable model (`small.en` on Apple Silicon before a bench) |
| `unknown` | RAM or CPU facts could not be read |

The status field `recommended` (a boolean) is separate from the suitability value: it marks the class default, the model `biorouter dictation install` picks with no argument, and it never changes automatically. A person may insist on any model that is not `unsupported_cpu`; nothing picks a slow model for them. On Settings the warning before a `too_slow` install is MSG-TURBO-CPU, and at a terminal the same sentence is a `[y/N]` question that `--yes` answers.

Cores are physical cores (`hw.physicalcpu` on macOS, distinct `physical id` and `core id` pairs in `/proc/cpuinfo` on Linux, `RelationProcessorCore` entries on Windows), falling back to the logical count where the system does not say: two hyperthreads do not run two encoders at full speed. RAM is total physical memory (`hw.memsize`, `MemTotal`, `GlobalMemoryStatusEx`).

`biorouter dictation bench` and **Test speed** (`POST /dictation/bench`) decode the first 5.5 s of an embedded copy of `jfk.wav` (11.0 s, 352,078 B, `include_bytes!` under the `engine` feature) with the active model, DD-32's threads and the detector on, once to warm and once to time. The real time factor is the timed decode's wall time divided by 5.5. The clip is a dictated sentence's length, not the whole sample, because the encoder always runs Whisper's full 30 s window: turbo on 8 CPU threads takes about 4.5 s whatever the audio's length [M], so the 11 s clip would rate it 0.41, fast enough, where a 5 s sentence rates it 0.9 (the M1 design's latency finding 2). The result goes to `Paths::in_local_data_dir("dictation/bench.json")` as `{<model id>: {rtf, elapsed_ms, audio_ms, threads, backend, measured_at, version}}`; a record from another Biorouter version, backend or thread count is ignored. A `serve` browser sees the host's figures.

Measured with PR-11's debug CLI on this M4 Max, Metal, 8 threads, warm Metal cache (2026-11-06) [M]: `whisper-small.en-q8_0` decoded the 5.5 s clip in 89 ms (real time factor 0.016) and turbo in 275 ms (0.050); both rate `recommended`.

### Language

`BIOROUTER_DICTATION_LANGUAGE` is `en` (default), `auto` or an ISO 639-1 code (DD-15). The CPU tier is English only in v1 (O-13). A `.en` model refuses any other value with MSG-LANGUAGE-EN, at `select`, at stream creation and in `transcribe --language`. A code whisper.cpp does not know is refused before a worker sees it (`LanguageUnsupported`); the list is `protocol::WHISPER_LANGUAGES`, which a test reads back from the vendored `g_lang` table. The two families use different vocabularies (`small.en` has 51,864 tokens; turbo is multilingual), so the token prompt is encoded per loaded model, never shared [M]. A multilingual `small` model can join later as a catalog change.

### Not shipped in v1

| Model | Why not |
|---|---|
| fp16 and q8_0 files of turbo; f16 of `small.en` | No accuracy gain over the quantized files [M]. Turbo q8_0 encodes 2.8 times faster than q5_0 on a CPU [M], which matters only if turbo is offered on CPU hosts; M-10 decides |
| `whisper-small.en-q5_1` | No more accurate than q8_0, and its encoder needs 2.8 times the single core work on a CPU [M]; on Metal the two run at the same speed |
| `base.en`, `medium` | Not needed by any class in v1; a later catalog change |
| `Qwen3-ASR` 0.6B and 1.7B | 0.6B wrote the wrong gene symbol (`EGLN1 encodes` became `EPAS1 encodes`) in 2 to 11 of 168 utterances and dumped the term list at 54 terms or more [M]; 1.7B Q8_0 exceeds the 2 GiB asset limit and needs `llama-server`, absent from the CLI packages and Docker [M, C] |
| Parakeet | A second engine; no prompt biasing; its weights are CC BY 4.0 whatever a conversion's card says [R] |

Adding any of them later is a catalog change shipped in an app release, with its own release in `biorouter-models`.

## In house hosting and the release process for model files

### The repository

Speech models are hosted in a new public repository, `BaranziniLab/biorouter-models`, with GitHub's immutable releases turned on (DD-16, R7). The hosting was decided by the owner on 2026-11-05; creating the repository, turning immutable releases on, naming who may publish, merging its workflow and publishing the first releases is owner action A-1, before PR-11 merges.

| Choice | Reason |
|---|---|
| Not releases of `BaranziniLab/biorouter` | The app's updaters read `releases/latest` of that repository, and `scripts/release.sh` asserts exactly 11 assets per release [C `release.sh:285`] |
| Not Git LFS | 10 GiB of bandwidth a month on the Free plan [R], about 18 downloads of the default model; BaranziniLab is on the Free plan [M] |
| Not GitHub Pages | 1 GB per site and 100 GB a month [R]; not a model host |
| Not a Hugging Face organization | Still a third party's site (R7) |

GitHub limits that shape the design: each release asset must be under 2 GiB, a release can hold 1,000 assets, and there is no limit on total size or bandwidth [R]. An immutable release can never gain an asset, so there is one release per file revision. The unauthenticated REST API allows 60 requests an hour per address [M], and many people on UCSF networks share one NAT address, so the app never calls `api.github.com` at run time.

Model tags are `stt-<id>-r<n>`: `stt-whisper-large-v3-turbo-q5_0-r1`, `stt-whisper-small.en-q8_0-r1` and `stt-silero-vad-v6.2.0-r1`. Each asset keeps its upstream file name, so names and hashes match upstream one to one. The app pins URLs of this form:

```text
https://github.com/BaranziniLab/biorouter-models/releases/download/stt-whisper-large-v3-turbo-q5_0-r1/ggml-large-v3-turbo-q5_0.bin
```

Never `releases/latest`. A download answers 302 to a signed URL on `release-assets.githubusercontent.com` that expires about an hour after it is issued, and that host answers `Range` requests with 206 and an ETag [M]. Release downloads redirect only to `release-assets.githubusercontent.com` today [M]; `objects.githubusercontent.com`, GitHub's former asset host, stays on the allowed list of final hosts as a precaution.

### Release contents

Each release carries:

| Asset | Contents |
|---|---|
| the model file | Byte identical to the upstream file at the pinned revision |
| `provenance.json` | `id, file, bytes, sha256, sha1, format, acquired_from{url@revision, hf_lfs_sha256, upstream_published_sha1, acquired_at}, derived_from, license{spdx, holders, files}, published{repo, tag, github_digest}` |
| `SHA256SUMS` | One line in `sha256sum` format |
| `LICENSE-openai-whisper.txt` and `LICENSE-whisper.cpp.txt` (or `LICENSE-silero-vad.txt`) | The license texts; the detector's release carries the Silero text |
| `NOTICE.md` | Who made the weights, who converted them, who hosts them |

An example `provenance.json` for turbo:

```json
{
  "id": "whisper-large-v3-turbo-q5_0",
  "file": "ggml-large-v3-turbo-q5_0.bin",
  "bytes": 574041195,
  "sha256": "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
  "sha1": "e050f7970618a659205450ad97eb95a18d69c9ee",
  "format": "ggml legacy (magic 0x67676d6c), q5_0",
  "acquired_from": {
    "url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
    "hf_lfs_sha256": "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    "upstream_published_sha1": "e050f7970618a659205450ad97eb95a18d69c9ee",
    "acquired_at": "<UTC time of the publishing run>"
  },
  "derived_from": {"original": "OpenAI large-v3-turbo checkpoint", "conversion": "upstream convert script, then quantize q5_0"},
  "license": {"spdx": "MIT", "holders": ["OpenAI (2022)", "The ggml authors"], "files": ["LICENSE-openai-whisper.txt", "LICENSE-whisper.cpp.txt"]},
  "published": {"repo": "BaranziniLab/biorouter-models", "tag": "stt-whisper-large-v3-turbo-q5_0-r1", "github_digest": "sha256:<read back after publishing>"}
}
```

The published artifact is a byte identical copy of upstream, not a reconversion: anyone can check it against two independent public records, the Hugging Face LFS SHA-256 and the upstream README's SHA-1 [M for both on the default files]. A reconversion from OpenAI's checkpoints is an optional audit only, because byte identity across converter versions is unverified.

### Publishing

The repository holds `catalog/<id>.json`, `.github/workflows/publish-model.yml`, its own copy of the retired name guard (R-31), the license texts and a README model card. `publish-model.yml` is the only thing that ever touches Hugging Face; no person and no app does at run time. Its upstream sources: the ggml set at Hugging Face revision `5359861c739e955e79d9a303bcbc70fb988958b1` (frozen since 2024-10-29 [M]), and the detector from `ggml-org/whisper-vad` at the revision recorded in `catalog/silero-vad-v6.2.0.json`.

1. A pull request adds or changes `catalog/<id>.json`: the source URL with its revision, the expected SHA-256, SHA-1 and size, the license ids and the tag.
2. On merge, `publish-model.yml` runs on an ubuntu runner. It downloads from the pinned revision and checks SHA-256, SHA-1, size and the legacy ggml magic (`lmgg` on disk).
3. It writes `provenance.json` and `SHA256SUMS`.
4. It runs `gh release create stt-<id>-r<n> --draft`, uploads every asset of [Release contents](#release-contents), then publishes the draft. The release is immutable from then on.
5. It reads back `.assets[].digest` and compares it with the expected SHA-256, then runs `gh release verify-asset` on each asset.
6. A Biorouter pull request then changes the compiled catalog. Its offline unit tests run in `rust.yml` automatically.

The same repository holds the Tier 1 evaluation set of [testing and release](testing-and-release.md). Evaluation tags are `eval-<name>-r<n>`, each with its own `catalog/eval-<name>.json` and the same workflow, which skips the ggml magic check for them. The first is `eval-primock57-subset-r1`: a FLAC tarball, `ATTRIBUTION.md` and `SHA256SUMS`. The app never downloads an evaluation release; only the accuracy workflow reads it. It must exist before PR-12 merges, because PR-12's first Tier 1 run reads it. `require_dictation_accuracy`, which PR-12 adds to `release.sh` `cmd_publish`, stays inert with a warning until the first Tier 1 run on a default model is green, and from then on a red run blocks `release.sh publish`.

A-1 is accepted when each of the three pinned releases is immutable, `gh release verify-asset` passes on every asset, and the reported digests equal the pins in [Catalog](#catalog). `eval-primock57-subset-r1` is published the same way before PR-12.

### Verification and the nightly pin check

The app trusts its compiled pins, never GitHub's reported digest at run time (DD-17). A new nightly workflow in the app repository, `.github/workflows/dictation-model-pins.yml`, compares every catalog pin with the `digest` GitHub reports for its asset (with the workflow's `GITHUB_TOKEN`) and runs `gh release verify-asset`. A red run means a release changed or a pin is wrong; neither should be possible with immutable releases, which is why it is checked.

### Licenses and `NOTICE`

License texts travel with the files (DD-18). They are release assets, they are compiled into the binary from `crates/biorouter-dictation/licenses/*.txt` with `include_str!`, and the installer writes them beside every installed file, so a model copied in with `--from` also carries its notice. The weights are MIT (OpenAI, 2022), the ggml conversions MIT (the ggml authors) and the detector MIT (Silero Team) [R].

`NOTICE` gains three entries in the existing style [C `NOTICE`]: whisper.cpp (the engine compiled into `biorouter` and `biorouterd`, MIT, at the pinned commit), miniaudio (the capture code in `biorouter`, Unlicense or MIT No Attribution) and "Speech models downloaded on request" (the Whisper weights, their ggml conversions and the Silero detector, all MIT, hosted at `BaranziniLab/biorouter-models`). The existing llama.cpp entry says "build b9611" while the pin is `LLAMA_SERVER_BUILD = "b11429"` [C `NOTICE`, `llamacpp_sidecar.rs`]; PR-11 corrects it and adds a test that pins the `NOTICE` versions to the code constants. A license that is not permissive (CC BY NC, the NVIDIA Open Model License and similar) is never hosted as a default; a CC BY or Apache file added later carries its attribution and a change notice.

### Mirrors and blocked networks

No UCSF mirror in v1 (DD-25, O-25). An administrator may set `BIOROUTER_DICTATION_MODEL_MIRROR=<https base URL>` (env only). The installer then tries `<mirror>/<release_tag>/<file_name>` first, over https only, and falls back to GitHub on any failure. The same compiled hashes apply, so a mirror can never change which bytes are accepted. Hospital proxies may block `release-assets.githubusercontent.com`; the operator documentation names both asset hosts to allow, and MSG-OFFLINE names the air gapped route. `HTTPS_PROXY` and `NO_PROXY` are honored through `reqwest`; the macOS system proxy setting is not read [I], so the manual checklist installs a model on a UCSF network behind its proxy.

## Installing a model on every OS and deployment

### Who installs, surface by surface

No speech model is part of an install or an update; a person chooses to install one (R6, DD-19 to DD-21).

| Surface | How a person opts in | Authority |
|---|---|---|
| S1 to S4 desktop app | The microphone's setup popover (**Install**) or **Settings > Models > Speech models**; or `biorouter dictation install` | The desktop holds the user action proof |
| S5 to S7 `serve` | The browser's Settings; or `biorouter dictation install` on the host | The daemon secret on a keyless daemon, unless `biorouter serve --no-dictation-install`, `BIOROUTER_DICTATION_BROWSER_INSTALL=false` or managed policy forbids it (O-2, decided by the owner on 2026-11-05) |
| S8 CLI package on a server | `sudo -u biorouter -H biorouter dictation install`, or `sudo biorouter dictation install --system` | A person at a terminal |
| S9 to S11, S19 terminal | `biorouter dictation install`, `/dictate install` | A terminal confirmation, or `--yes` without a terminal; refused when `BIOROUTER_TERMINAL=1` even with `--yes` (MSG-INSTALL-SHELL) |
| S12 ACP | None over ACP; the desktop or the CLI on that computer | as those surfaces |
| S13 Jupyter AI | Not offered: the owner excluded Jupyter from dictation (O-30) | n/a |
| S14 Crew | Each member installs locally; models never cross the broker | as the member's surface |
| S15 apps | The serving daemon's models; an exported app never contains a model, and a launcher that sets `BIOROUTER_PATH_ROOT` starts with an empty per user folder | as the serving daemon |
| S16 Docker | `docker run … <image> dictation install` with a data volume, the browser through `serve`, or an administrator's derived image (`RUN biorouter dictation install --system --yes whisper-small.en-q8_0`) | a person, or the image's author |
| S17 external backend | Settings on the laptop, with that server's **User Action Key** on a keyed daemon | the backend's rules |
| S18 clipboard | The local folder, as the desktop or CLI | as those surfaces |
| S20 air gapped | `biorouter dictation install --from <file\|dir>`, or files placed in the folder and **Rescan** | as the surface |

Agents never install (DD-20, P-7): no tool installs, removes or selects a model, `biorouter dictation install` refuses under `BIOROUTER_TERMINAL=1`, and on a keyed daemon the route needs the proof. This prevents accidents by a cooperative agent; it does not stop one that unsets the variable. On a keyless daemon a refusal would be a dead control, and the route chooses only whether, never what: a compiled id and its hash.

### Where models go (O-1)

"Installation location" is read as Biorouter's own per user data folder for the account that runs Biorouter, plus an administrator's system folder on servers (DD-19, O-1, decided by the owner on 2026-11-05). Inside the app is impossible on macOS, where writing into the signed bundle breaks its seal and every update replaces the bundle [I], and in the Linux packages, where `/usr` belongs to the package manager. On Windows it is possible: beside Squirrel's versioned `app-<version>` folder, which each update replaces while the folder around it survives, or in the folder where a person unpacked the zip. But an uninstall deletes the first, and the zip has no fixed location [I from Squirrel's layout, not measured]. The per user data folder behaves the same way on every OS, and the desktop app and the CLI share one copy there.

Where speech models go, per OS and deployment (O-1). In every case the folder is created by the installer, shown with the account in **Settings > Models > Speech models** (**Model folder** row) and printed by `biorouter dictation status`.

| Deployment | Account that owns the files | Folder | How a person installs |
|---|---|---|---|
| Desktop app, macOS (S1, S2) | The person | `~/.local/share/biorouter/dictation/models/<id>/` | Setup popover or Settings; or `biorouter dictation install` |
| Desktop app, Linux (S4) | The person | `~/.local/share/biorouter/dictation/models/<id>/` | As above |
| Desktop app, Windows (S3) | The person | `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\<id>\` | As above |
| Any of the above with `BIOROUTER_PATH_ROOT` set | The person | `<root>/data/dictation/models/<id>/` | As above |
| `biorouter serve` (S5 to S7) | The account that runs `serve` | That account's data folder, as above | The browser's Settings with the secret (O-2), or `biorouter dictation install` on the host |
| CLI only package run by systemd (S8) | The service account, `biorouter` | The service account's data folder, `~biorouter/.local/share/biorouter/dictation/models/` | `sudo -u biorouter -H biorouter dictation install` |
| Any server, shared by every account | root installs; every account reads | `/var/lib/biorouter/dictation/models/` (Linux), `/Library/Application Support/Biorouter/dictation/models/` (macOS), `%ProgramData%\Biorouter\dictation\models\` (Windows) | `sudo biorouter dictation install --system` |
| A managed fleet | The administrator | The managed `dictation.models_dir`, read only, still hash checked | Files placed there by the administrator |
| Docker (S16) | `biorouter` in the container | A data volume at `/home/biorouter/.local/share/biorouter`, or a read only mount at `/var/lib/biorouter/dictation/models` | `docker run … <image> dictation install`, or the browser's Settings through `serve` |
| WSL (S19) | The WSL Linux account | `~/.local/share/biorouter/dictation/models/` inside WSL | `biorouter dictation install`, or `--from /mnt/c/…` to reuse a Windows side download |
| External backend (S17) | The account that runs the backend | The backend host's folder, not the laptop's | Settings on the laptop with that server's key, or the host CLI |
| Air gapped hosts (S20) | As the surface | As the surface | `biorouter dictation install --from <file\|dir>`, or files placed in the folder and **Rescan** |

On Windows the folder is under a new local root, not Roaming: a 0.3 to 0.6 GB file in Roaming would be copied at every logon on a roaming profile [I]. PR-06 adds it:

```rust
// crates/biorouter-mcp/src/paths.rs
pub struct PlatformDirs { pub config: PathBuf, pub data: PathBuf, pub state: Option<PathBuf>,
                          pub cache: PathBuf, pub local_data: PathBuf }   // new field

pub fn windows_dirs(roaming_base: &Path, local_base: &Path) -> PlatformDirs {
    let roaming = AppRoot::under(roaming_base).resolve().to_path_buf();
    let local = AppRoot::under(local_base).resolve().to_path_buf();    // legacy folder rules apply
    PlatformDirs { config: roaming.join("config"), data: roaming.join("data"), state: None,
                   cache: local.join("cache"), local_data: local.join("data") }
}
// macOS and Linux: local_data == data.

// crates/biorouter/src/config/paths.rs
impl Paths {
    pub fn local_data_dir() -> PathBuf;              // honors BIOROUTER_PATH_ROOT as <root>/data
    pub fn in_local_data_dir(subpath: &str) -> PathBuf;
}
```

Today `windows_dirs` puts `data` under the roaming root and only `cache` under the local root [C `crates/biorouter-mcp/src/paths.rs`]. PR-06 adds a row to `crates/biorouter/tests/path_resolver_agreement.rs`, and to `app_folder/resolution-cases.json` if the legacy folder move must cover the new root. The models folder is `Paths::in_local_data_dir("dictation/models")`. An id stored in configuration needs no rewrite after the legacy folder move.

### Lookup order and writers

For each catalog id, the first verified copy wins:

1. The managed `dictation.models_dir`. When managed `model_downloads: false`, it is the only source.
2. The env only `BIOROUTER_DICTATION_MODELS_DIR`.
3. The per user folder.
4. The system folder.

In every root a file is found at `<root>/<id>/<file_name>`, or at `<root>/<file_name>` in the read only roots (1, 2 and 4) so an administrator can drop files in flat. A copy whose hash is wrong is skipped and left in place, and the next root is tried.

| Writer | Writes |
|---|---|
| The daemon and `biorouter dictation install` | Only the per user folder |
| `sudo biorouter dictation install --system` | Only the system folder, with 0755 folders and 0644 files; when an administrator's managed `models_dir` names that same folder (the example in [The `dictation:` block](#the-dictation-block) does), this is how they fill it. Before it writes or deletes anything there, `--system` checks the path (MSG-UNSAFE-FOLDER otherwise): on Unix every existing folder on the way, as written and as resolved, must be owned by root and writable by no other account (a sticky folder such as `/tmp` passes), so no other account can put a link in its path while root writes; on Windows no part may be a link or a junction, and owners are not read, as in the managed policy's own check. Temporary files are created new after any stale one is unlinked, and the lock and the partial are opened without following a link (`O_NOFOLLOW` on Unix) |
| Nothing | The managed and env folders |

Under `BIOROUTER_PATH_ROOT` the system folder is `<root>/system/dictation/models`, so a sandbox never reads the computer's real one; an env folder that is not an absolute path is ignored. Status reports `location: user | system | managed | env`, the account and the folder. Settings shows **Remove** only for `user` copies; a `system` copy is removed with `sudo biorouter dictation remove --system`.

### The installer, step by step

One Rust installer in `crates/biorouter-dictation/src/models.rs` serves the daemon and the CLI (DD-21). The Electron download helper `utils/resumableDownload.ts` is unreachable from the CLI and `serve`, and it restarts every download from an empty file [C], so it is not reused.

1. Resolve the id through `STT_MODEL_CATALOG`, or through `RETIRED_STT_MODELS` to its replacement. An unknown id is refused.
2. Check the gates: the engine is compiled; managed policy allows dictation, downloads and this model; the host class is not `NO`; the caller may install (keyed daemon: the proof; keyless: not locked; CLI: a terminal or `--yes`, and never `BIOROUTER_TERMINAL=1`).
3. Plan the files: the model plus every `requires` entry (the voice detector) whose current revision is not already verified in some root. A copy of an earlier revision (its hash in `accepted_sha256`) counts as missing, so an install fetches the current one, and the earlier copy stays loadable until the new file is renamed over it (DD-23).
4. Check the disk: `fs2::available_space(folder)` must be at least the remaining bytes plus 256 MiB, else MSG-DISK. Nothing calls `fs2::available_space` today [C], and its answer on network home folders is unmeasured [I].
5. Take an exclusive `fs2` lock on `<models>/<id>/.lock` before anything else. If another process holds it, the daemon attaches to the running job and the CLI waits, printing that another Biorouter process is installing this model. If the current revision is already there and verifies, stop here.
6. For each attempt, request the mirror URL first when `BIOROUTER_DICTATION_MODEL_MIRROR` is set, then the github.com URL. Request the github.com URL again on every attempt, never a stored signed URL (they expire after about an hour [M]). Follow at most 5 redirects, https only, and accept only `release-assets.githubusercontent.com` or `objects.githubusercontent.com` as the final host. Send no `Authorization` header on any hop. An attempt reached the repository only when a host that serves the bytes (an asset host, or the mirror) answered; github.com's redirect alone does not count.
7. When a partial exists, send `Range: bytes=<len>-` with `If-Range: <etag>`. A 206 must answer with `Content-Range` starting at `len`, or the partial is truncated to 0. A 200 restarts from 0. A 416 hashes the partial: it installs when the hash is the current pin and deletes it otherwise. A 403 or an expired redirect requests the github.com URL again.
8. Back off on 429 and 5xx, honoring `Retry-After` up to 60 s. The retry budget counts only attempts that made no progress, and an attempt made progress only when the partial ends it longer than it began: five in a row stop the install. So a server that ignores `Range` and cuts every body at the same place stops after its first answer plus five, instead of downloading the same half forever. 4xx answers other than 408, 425 and 429, TLS failures, `ENOSPC`, `EACCES` and `EROFS` stop at once. A TLS failure is told by type, never by the words of a message: rustls's own error anywhere in the chain (a certificate, an alert from the server, bytes that are not TLS, as from a plain HTTP server or a proxy that rewrites the connection) stops the install with MSG-TLS; a handshake the network cut short (`UnexpectedEof`), a reset and an aborted connection are retried like any dropped connection.
9. Write to `<models>/.partial/<id>/<file>.partial`, on the same volume so the final rename is atomic, with `<file>.partial.json {url, etag, size, sha256}` beside it. A partial whose recorded `sha256` differs from the current pin is discarded. Publish progress through a `tokio::sync::watch` (the status `install_job`). A body that would run past the pinned size stops the install with MSG-HASH at once.
10. At the end, check the size, hash the whole partial once, and compare with the current pin. A mismatch deletes the partial and reports MSG-HASH. As built, the hash runs once, after the last byte, not also while streaming: a streamed digest would have to start by reading back whatever a resumed partial already held, and the final hash is the one that decides either way.
11. `fsync` the partial, rename it to `<models>/<id>/<file_name>`, write the license files and `installed.json`, and `fsync` the folder. Every handle the installer opened on a model, record or license file is closed before that file is renamed or deleted, and the lock is a file of its own, never the model; remove deletes the lock file while it holds it and closes it before it removes the folder. On Windows the flush opens the file for writing, because `FlushFileBuffers` answers a read only handle with "Access is denied" (the first Windows CI run failed every install there); the folder is not synced, since NTFS journals the rename; and the flush's open, the final rename, each record's rename and a removal's delete are tried again for up to about 2 s while they fail with "Access is denied" or a sharing violation, which is what a scanner holding a file it has just seen looks like. macOS and Linux run each of them once.
12. Release the lock and set the job's phase to `installed`.

The steps that hash a file or `fsync` it (the disk check, which may hash a copy that has no record, the check in step 5, the 416 hash, the final hash and the publish) run on blocking threads: on the daemon's runtime the same threads serve dictation streams, and a 574 MB hash would hold one for over a second. A connection that drops in the middle of a body is progress when it added bytes, so the same install resumes at once from the bytes it has. Cancel keeps the partial, so a later install resumes. In the CLI, Ctrl-C cancels at once wherever the install waits (a connection, a backoff, another process's lock) and prints MSG-INSTALL-CANCELLED. Remove takes the model's lock first and refuses while another process holds it (MSG-REMOVE-BUSY), because deleting a running download's partial would fail that install at its end and unlinking its lock would let a third install run beside it; it then deletes the final file, its record and licenses, its partial and its metadata, a staged `--from` copy and any temporary file, and the lock file last. Every network error ends in a sentence: MSG-OFFLINE when no host that serves the bytes could be reached, which is also what a network that lets github.com through and blocks the asset hosts gives, MSG-DOWNLOAD-FAILED when one answered and five attempts in a row added nothing, MSG-TLS, MSG-HASH, MSG-DISK. The daemon runs one install job at a time; a second request for the same id attaches, and a request for another id while a job runs answers 409.

`install --from` identifies every file by its SHA-256 first, then checks the disk for all the files it will copy (MSG-DISK), then copies each one, hashing it again as it is written. A copy that fails for any reason (a full disk, a read error, a hash that changed) is deleted, so an interrupted import never leaves a staged file that no command can see. A read error names the file it could not read; a write error says the model could not be written, or is MSG-DISK on a full volume.

### `installed.json` and verification on load

```json
{"id": "whisper-small.en-q8_0", "sha256": "67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067", "size": 264477561, "mtime": "2026-11-04T18:20:11Z", "verified_at": "2026-11-04T18:20:13Z", "source": "download", "release_tag": "stt-whisper-small.en-q8_0-r1"}
```

`mtime` is written with nanoseconds and compared as text, so a touch within the same second still forces a hash. `source` is `download`, `mirror`, `import` (`--from`) or `rescan`; a copy found in its `<id>/` folder with no record (put there by hand) is hashed and gets one with `source: rescan`. A file is verified by SHA-256 before its first parse (DD-17, P-11). Before every `load`, the service compares the file's size and mtime with `installed.json` and hashes it again on any difference; a per user file that then fails is deleted and reported as not installed. Files in the read only roots have no `installed.json`; each process hashes them once and caches the result by path, size and mtime, and never deletes them. A file whose hash matches no pin and no `accepted_sha256` is never loaded, and status lists it as unrecognized. A `.partial` file is never loaded.

### Terminal commands

The model commands land in PR-10 (`status`) and PR-11 (the rest). The full CLI tree is in [surfaces](surfaces.md).

```text
biorouter dictation status   [--format text|json] [--verbose]
biorouter dictation list     [--format text|json]                          (alias ls)
biorouter dictation install  [MODEL] [--from FILE|DIR] [--system] [--yes] [--suggest-from-chats | --no-suggest-from-chats]   (alias pull; the two suggest flags arrive with PR-23)
biorouter dictation remove   MODEL [--system] [--yes]                      (alias rm)
biorouter dictation use      MODEL
biorouter dictation urls     [MODEL]
biorouter dictation verify
biorouter dictation bench    [--format text|json]
```

| Command | Behavior |
|---|---|
| `status` | Engine compiled, the CPU result and its sentence, backend, threads, models found with their `location`, policy, account, data folder. `--format json` is schema 1 and adds `capture: {available, reason}` for the terminal's own capture. From PR-10, before capture exists, it answers `available: false, reason: "not_compiled"`; from PR-25, which brings capture and its audio system probe, the CLI package containers answer `available: false, reason: "no_audio_system"`. `--verbose` starts a worker, prints its `hello`, and prints the kept stderr lines if it failed |
| `list` | Every catalog model with size, languages, suitability, whether installed and where; retired and unrecognized files with their sizes |
| `install` | With no `MODEL`, the class default. Prints model id, size, source (`github.com/BaranziniLab/biorouter-models`), SHA-256, target folder and account before asking; confirms at a terminal; needs `--yes` without one (exit 2); always refuses when `BIOROUTER_TERMINAL=1` with MSG-INSTALL-SHELL. `--system` needs root or Administrator (exit 2 otherwise). Progress follows `PullReport` in `commands/models.rs` [C]: cliclack on a terminal, plain lines otherwise |
| `remove` | Deletes, and removes the detector with the last speech model in that folder; the detector stays while another speech model there needs it or is being installed by another process (it holds that model's lock: such an install planned its files while the detector was there and does not fetch it again), and `remove` given the detector's own id refuses then with MSG-DETECTOR-NEEDED (exit 2); never refused by policy. It refuses while another process installs the same model (MSG-REMOVE-BUSY). The CLI runs no worker, so it has nothing of its own to unload; the daemon's remove route unloads its worker first (PR-15). Refused, like `install` and `use`, when `BIOROUTER_TERMINAL` is set, because an agent never removes or selects a model either (P-7) |
| `use` | Writes `BIOROUTER_DICTATION_MODEL`; refuses an unknown or uninstalled id, a model managed policy leaves out, and a `.en` model while the language is not `en` |
| `urls` | URL, size and SHA-256 of the model and every file it requires ([Air gapped hosts](#air-gapped-hosts)) |
| `verify` | Hashes every installed file in every root again; deletes failing per user files and reports the rest; exit 1 on any failure |
| `bench` | The bench of [Suitability and bench](#suitability-and-bench) |
| `warm-up MODEL` (hidden) | One load and one decode, then the worker stops; `install` starts it in the background (see below) |

Every refusal that means "nothing was attempted" (an agent's shell, no terminal without `--yes`, `--system` without root or Administrator, managed policy, an unknown id, no engine, an unsupported processor) exits 2 with its sentence alone; a failed download, hash or verification exits 1. While managed `model_downloads: false` makes the managed folder the only source, `install --from` into any other folder is refused with MSG-POLICY-DOWNLOADS too, because a copy there would never be read; `--system` into a system folder that is the managed `models_dir` is allowed. Before turbo on a `CPU` host with no bench record, the question names the rule rather than a time, because MSG-TURBO-CPU needs a measured time that does not exist before the model is installed; `bench` measures it afterwards.

**The warm up after an install (latency).** On Metal the first load of a model after an install compiles its shaders into the per user Metal cache, 3.0 to 4.2 s of the first dictation [M]. So `install`, once it has placed a speech model, starts `biorouter dictation warm-up <id>` as a detached process (its own process group on Unix, no console on Windows) and returns without waiting for it; an install that only waited on another process's lock, or that fetched only a missing voice detector, starts none, because the model is already being or has been prepared; the warm up loads the model and decodes 2 s of the bench clip, and every later process finds the shaders compiled. On a CPU host there is nothing to compile and a turbo decode would burn seconds of CPU, so nothing starts there. The daemon's install route (PR-15) calls `DictationService::warm_up` on a task of its own after a verified install, which also leaves the worker loaded for the first dictation until the idle unload. Measured in the PR-11 live check on this M4 Max with a warm Metal cache (debug CLI, three runs each, 2026-11-06) [M]: `whisper-small.en-q8_0` ready in 229 to 238 ms (load 68 to 70 ms, encoder warm up 55 to 62 ms) and the 2 s decode in 75 to 76 ms; turbo ready in 456 to 478 ms (load 101 to 107 ms, encoder warm up 249 to 250 ms) and the decode in 261 to 262 ms. The lookup that turns an id into the files to load (`resolve`: the managed policy, then the folders, with `installed.json` in place of a hash in this account's folder) took under 1 ms each time with the copies in this account's folder. It looks up only what it will load, after the policy and language checks: the model and its detector for a named id, and with no id the candidates in order (the configured model, the class default, catalog order) until the first usable one, so a model nobody asked for is never opened. That matters in the read only folders (managed, env, system), whose copies each process hashes once: when `resolve` read every folder whole, `biorouter dictation use whisper-small.en-q8_0` with the catalog files in a sandbox's system folder took 5.29 s in the debug CLI, and 16.43 s with turbo there too. As built it takes 2.5 to 2.75 s in the debug CLI with or without turbo, and 0.19 to 0.21 s in the release CLI, process start included (three runs each, 2026-11-06) [M]; the daemon pays that once per process, and only for the files it loads. The cold cache figure needs no cache deleted: the per user Metal cache is keyed by the executable, not by the shader source or the path, so every new build compiles again. The first `transcribe` of a freshly built release `biorouter`, with both models installed hours before, had the model ready after 2,972 ms, of which the load was 62 ms and the rest the worker's start, where the backend compiles; the next run was ready after 156 ms, and a byte identical copy of that build at another path compiled nothing (ready after 181 ms) (2026-11-06, M4 Max) [M]. So an app update costs the first dictation after it about 3 s unless something warms the new build first, which the install warm up does not do. After `install --system`, or any install run as root or an Administrator, nothing is warmed: the Metal cache belongs to the account that compiles, so a warm up run as root would prepare nothing for the people who dictate, and the CLI prints MSG-WARM-UP-SYSTEM instead.

`biorouter doctor` gains a **Dictation** section between "Local models" and "Shell sandbox" and a `dictation` JSON key, with no `doctor --fix dictation`. On a host where a running `biorouterd` belongs to another account (the systemd unit's `User=biorouter` [C `docs/deployment/headless-linux.md`]), `install` and `status` print MSG-OTHER-ACCOUNT, because models installed under the person's own account are not visible to that daemon.

### Air gapped hosts

No HTTP route takes a filesystem path, and there is no `/dictation/models/import` (DD-24). A route that takes a path would answer whether a file exists and would add a parser entry for any file the caller names.

1. On a connected machine, run `biorouter dictation urls [MODEL]`. It prints each file's URL, size and SHA-256:

```text
whisper-small.en-q8_0  264,477,561 B  sha256 67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067
  https://github.com/BaranziniLab/biorouter-models/releases/download/stt-whisper-small.en-q8_0-r1/ggml-small.en-q8_0.bin
silero-vad-v6.2.0  885,098 B  sha256 2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987
  https://github.com/BaranziniLab/biorouter-models/releases/download/stt-silero-vad-v6.2.0-r1/ggml-silero-v6.2.0.bin
```

2. Carry the files to the host.
3. Run `biorouter dictation install --from <file|dir>`. It identifies each file by SHA-256 against the catalog, not by name, so a renamed file works; it copies the file into place (the system folder with `--system`), writes the license files and `installed.json` with `source: import`, and lists any file it did not recognize.
4. On the desktop or `serve`, put the files in the model folder (**Show in Finder** or **Show in Explorer** on the desktop) and press **Rescan**.

`POST /dictation/models/rescan` takes no arguments. It looks at the files at the top level of every models folder this account reads, hashes those whose size equals a catalog size, places each match in the per user folder at `<id>/<file_name>` with its license files and an `installed.json` with `source: import` (the copy `install --from` makes [C `models.rs` `import_from`]), and answers `{added, unrecognized}`, where `unrecognized` lists file names only. A file it placed from the top of the per user folder itself is moved: the original is removed once its copy is verified in place, so the model is not kept twice (before this, the three catalog files stayed beside their placed copies, 840 MB held twice [M, 2026-11-06]). A speech model it placed is warmed up in the background, as after an install, so the first dictation does not pay the Metal compile (a first load that compiled turbo's Metal pipelines took 3,459 ms to a ready model, against 509 to 805 ms once they were cached [M, 2026-11-06]). When the host cannot reach the repository, install reports MSG-OFFLINE, which names exactly this procedure. Fleets use the managed `models_dir`. Transcription itself needs no network on any host.

### Packaging guards that keep models out of every artifact

| Guard | Fails when |
|---|---|
| `ui/desktop/scripts/prepare-platform-binaries.js` (PR-11) | Any file under `ui/desktop/src/bin` starts with the legacy ggml magic. Everything in `src/bin` lands inside the app bundle [C `forge.config.ts` `extraResource`] |
| `scripts/build-cli-linux-packages.sh` (PR-10, PR-11) | `biorouter dictation status --format json` in the package containers reports any installed model (PR-10's `--expect-no-models`), or a packaged file starts with the ggml magic (PR-11's scan in both containers) |
| The R6 guard in the package smokes and `release.sh verify` | A ggml magic file in either CLI package or the Docker image, or `dictation status` reporting an installed model in any package smoke |
| The nightly Docker workflow (PR-31) | The image contains a model file |
| `no_model_is_embedded`, a `--lib` source test in `crates/biorouter-dictation` (PR-11) | Any `include_bytes!` or `include_str!` outside `#[cfg(test)]` in `crates/biorouter-dictation`, `crates/biorouter-dictation-capture` or `crates/biorouter/src/dictation` names a `.bin` file or a path under `vendor/whisper.cpp/source/models/`. The bench clip `samples/jfk.wav` is the one embedded file, and it is not a model |
| A size ceiling in every package smoke (PR-11) | `biorouter` or `biorouterd` grew past its ceiling: its v1.94.1 size plus the growth since plus slack. `dictation_size_ceiling` in `scripts/smoke-test-release-artifacts.sh` holds the six ceilings: the v1.94.1 size of each binary (Linux x86_64 187,413,248 and 176,920,720 B; macOS arm64 164,389,760 and 154,775,664 B; macOS x86_64 176,538,336 and 166,285,184 B, read from the published release [M]) plus the growth since. PR-09's spike figure alone was too small: this branch's macOS arm64 release build at PR-11 grew by 9,613,412 B (`biorouter`) and 8,306,152 B (`biorouterd`) over v1.94.1 with its signature removed [M], about 2.9 MB and 1.6 MB past the 6.69 MB of the table above, which includes the 352,078 B bench clip; that leg has 1 MiB of slack. Linux and Intel macOS were not built, so they take PR-09's figure for the target plus the same excess [I], with 32 MiB of slack until the first CI cross build measures them, so a wrong estimate cannot fail a release; 32 MiB still hides no speech model. `smoke_mac`, `smoke_deb`, `smoke_rpm` and `smoke_cli_packages` check them, and also read every file the package installs (on Linux the package's own `dpkg -L` or `rpm -ql` list, on macOS the whole bundle) for the ggml magic. Windows has no ceiling yet, because no smoke in this script runs the Windows artifacts |

The magic checks above catch a model as a file. A model compiled into a binary with `include_bytes!` passes them, and the committed detector fixture is byte identical to the production detector, so the last two rows cover the binaries. Scanning a binary for the ggml magic cannot work, because the loader itself holds that constant. The source test catches an embedded file by its path whatever its size, in any shape rustfmt leaves it: it reads each file as one text, follows an `include_bytes!` argument across lines to its closing bracket, and leaves out only the items behind `#[cfg(test)]`. The size ceiling catches any speech model however it got in, since the smallest is 264 MB. It cannot catch the 885,098 B voice detector, which fits in any leg's slack, so the source test is the detector's only guard.

The release asset count stays exactly 11; `release_assets` and the updaters are untouched [C `scripts/release.sh`]. The 885,098 B voice detector is not bundled either; it downloads with the first speech model (DD-26, O-26).

### The install box and its terminal equivalent

From PR-23 on, the first speech model install also offers **Suggest dictionary words from my chats**, ticked and visible beside **Install** in the setup popover and in Settings (O-33, decided by the owner on 2026-11-05), with MSG-SETUP-SUGGEST below it (DD-56). At a terminal, `biorouter dictation install` asks "Suggest dictionary words from your chats? [Y/n]"; `--yes` without `--suggest-from-chats` leaves the capability off. After the first verified install the marker `dictation_dictionary_activation_version: 1` is written whatever the box says; the `dictionary` capability is turned on for new chats only when the box was ticked. A later "off" sticks, and nothing turns it on at daemon or CLI start. `POST /dictation/models/install` never carries the box: its body is `{model}` only, because on a keyless `serve` daemon any holder of the secret may install a model (DD-20), while turning on harvesting is the person's choice alone (DD-56). After a verified install the interface sends the box's value to `POST /dictionary/activation {suggest_from_chats}` with `userActionHeaders()` (specified in [dictionary](dictionary.md)), and that route calls `crates/biorouter/src/dictation/activation.rs`. On a keyed daemon it needs the proof; on a keyless daemon `false` is admitted and `true` is refused with MSG-DICTIONARY-HOST-SETTINGS, so the box is disabled before the click there. Before PR-23 neither the box nor the question exists, because a control appears only in the pull request that makes it work (DD-94).

Measured in PR-11's live check (2026-11-06, M4 Max, debug CLI) [M]: `biorouter dictation install --yes whisper-small.en-q8_0` downloaded both files from A-1's releases through github.com's redirect and installed them in 11.3 s, the debug build's SHA-256 of the 264 MB file included; `verify` took 5.4 s, all of it the debug hash. A release build of this branch installed all three catalog files with `--from` (each hashed to identify it and again as it was copied) in 4.8 s and `verify` hashed their 840 MB in 2.65 s, about 320 MB/s, the rate of a software SHA-256: `sha2` 0.10 uses the ARMv8 SHA-2 instructions only with its `asm` feature, which nothing turned on. The leaf crate now uses `sha2` 0.11 (already in `Cargo.lock`), which detects them, and x86's SHA extensions, at run time. Measured again on the release CLI (2026-11-06, M4 Max) [M]: `verify` of the same 840 MB takes 0.45 to 0.47 s, process start included, about 1.8 GB/s, against 0.31 s for `openssl dgst -sha256` of turbo's 574 MB alone; `install --from` of the three files takes 4.35 s, most of it writing and `fsync`ing 840 MB of copies, since its two hashes now cost about 0.9 s. `status` and `list` took 0.06 s with the copies in this account's folder, because they and every load read `installed.json` and hash nothing; with the three files in a sandbox's system folder, `status`, which hashes every read only copy once per process, takes 0.46 s. The end to end check on the release CLI (2026-11-06, M4 Max, a fresh sandbox) [M]: `biorouter dictation install` with no model at a terminal picked turbo, listed the detector and turbo with their sizes and pins, asked once, and placed both in 14.8 s after Enter, about 14 s of it turbo's 574 MB arriving at about 41 MB/s on the first fetch; later fetches of the same files ran at 230 to 260 MB/s (small.en in about 1 s), so the asset host's own cache sets the first person's wait. The final hash and publish of turbo took under 0.35 s, `verify` of turbo and the detector 0.32 s, `status` and `list` 0.06 s. An install killed with SIGKILL at 109 MB, and one stopped with Ctrl-C at 82 MB, both resumed from their partial (`Range` with `If-Range`), never from 0. With no network (a proxy that refuses every connection), MSG-OFFLINE came after 15.2 s, the four backoffs of 1, 2, 4 and 8 s. With a warm Metal cache, `warm-up` had turbo ready in 444 to 458 ms and decoded in 259 to 260 ms; small.en was ready in 224 to 225 ms and decoded in 74 to 75 ms.

**Acceptance (PR-06, PR-11, A-1).** On a Mac, a Windows laptop and a Linux server, `biorouter dictation install` then `verify` succeed and `status` lists the model as verified with its `location`; the installer, lookup order, suitability, catalog and command tests of [testing and release](testing-and-release.md) pass; `tests/model_download_live.rs` (`#[ignore]`) downloads the detector end to end from A-1's release; a planted model file fails the packaging guard; `BIOROUTER_PATH_ROOT=<r>` resolves the models folder to `<r>/data/dictation/models`.

## Removal and catalog updates

| Case | Behavior (DD-23) |
|---|---|
| Remove a speech model | Settings **Remove** (the proof on a keyed daemon, the secret on a keyless one) or `biorouter dictation remove MODEL`; never refused by managed policy, and refused while another process installs the same model (MSG-REMOVE-BUSY). The daemon's route unloads its worker first (PR-15), because on Windows a file opened without `FILE_SHARE_DELETE` (as MSVC `fopen` opens it) cannot be deleted while it is open; whisper.cpp closes the model file after loading, so the conflict exists only during a load. The CLI runs no worker of its own, so it has nothing to unload |
| Remove the last speech model | The voice detector is removed too, and the microphone returns to its "not set up" look |
| Reset and uninstall | Neither touches models: reset's categories are applications, knowledge, skills, extensions, schedules, workflows and history [C `routes/reset.rs` `ResetCategory`], and uninstalling the app or a package leaves the data folder. The operator documentation lists the folders to delete by hand |
| A new revision of a file | A new release `stt-<id>-r<n+1>` and a new pin in an app release. The old hash goes into `accepted_sha256`, so the installed file keeps working and status reports `superseded: true`; Settings offers **Update** and `biorouter dictation install <id>` fetches the new revision. Nothing updates a model automatically, and an app update never downloads one |
| A retired id | `RETIRED_STT_MODELS` maps it to its replacement for a configured `BIOROUTER_DICTATION_MODEL`; its files stay until the person removes them, and `list` shows them with their size |
| A file no pin recognizes | Never loaded; listed as unrecognized |
| The catalog itself | Changes only with an app release. There is no remote catalog |

## The administrator's switch

### The `dictation:` block

The managed policy file gains a `dictation:` block (DD-90, PR-05). The file lives at `/Library/Application Support/Biorouter/managed-policy.yaml` on macOS, `/etc/biorouter/managed-policy.yaml` on Linux and `%ProgramData%\Biorouter\managed-policy.yaml` on Windows [C `crates/biorouter/src/config/paths.rs` `managed_policy_path`].

```yaml
dictation:
  enabled: false               # no capture or transcription anywhere, no downloads, no dictionary capability
  capture: false               # no live microphones; transcribing a recording still works
  model_downloads: false       # only models already in models_dir are used
  allowed_models: [whisper-small.en-q8_0]
  models_dir: /var/lib/biorouter/dictation/models   # read only, still hash checked
  external_engine: false       # parsed in v1, no effect until an engine URL exists
  dictionary: false            # no dictionary edits, sidebar row, capability, extractor or bias
  dictionary_agent_writes: false   # agent tools not offered; extractor off; agent_writes pinned to suggest
```

Every key only narrows; `true` equals absent. With `dictionary: false`, transcription still works with an empty bias set.

### Parsing

Today a parse error anywhere in the file, or a failure to read it, sets `load_failed` and leaves an inert policy, which lifts every deny. An untrusted file is ignored with a warning and leaves the same inert policy without setting `load_failed` [C `crates/biorouter/src/managed/mod.rs` `load_from`]. The dictation block must not make that worse, and must fail closed for itself (O-27):

```rust
// crates/biorouter/src/managed/settings.rs
pub struct ManagedPolicyFile {
    // existing: hooks, allow_project_hooks, permissions
    #[serde(default)]
    pub dictation: Option<serde_yaml::Value>, // parsed apart, so a bad block cannot fail the file
}

// crates/biorouter/src/managed/dictation.rs (new)
pub const MANAGED_DICTATION_OFF: &str =
    "Dictation is turned off on this computer by your organization's managed policy."; // MSG-POLICY

pub struct Allowance { pub allowed: bool, pub reason: Option<String> }
pub enum Rule { Allowed, Forbidden(&'static str) }

pub struct DictationPolicy {
    pub managed: bool,
    pub enabled: Rule, pub capture: Rule, pub model_downloads: Rule,
    pub allowed_models: Option<Vec<String>>, pub models_dir: Option<PathBuf>,
    pub external_engine: Rule, pub dictionary: Rule, pub dictionary_agent_writes: Rule,
}

impl ManagedPolicy {
    pub fn dictation(&self) -> &DictationPolicy; // computed when this ManagedPolicy value is built
}
```

| File state | Result |
|---|---|
| No file | Unmanaged: every rule allowed, `managed: false` |
| A trusted file with no `dictation:` key | Unmanaged for dictation |
| A valid block | Each key narrows as written |
| An unknown key or a wrong type in the block | Dictation off, with a warning naming the key; the rest of the file still applies |
| A file that fails to parse, an unreadable file or an untrusted file | Dictation off, with a warning; the rest of the policy behaves as today |

A newer key on an older binary therefore turns dictation off rather than being ignored (O-27). `ManagedPolicy::load` reads, verifies and parses the file on every call, and nothing caches the result, despite its documentation saying "once" [C `crates/biorouter/src/managed/mod.rs` `load`]. Each caller reads it when it needs a verdict: `Agent::with_config` once per agent it builds (`agents/agent.rs`), the file operation check in `agents/extension_manager.rs` at every check, and `hooks/mod.rs` when it builds a hooks manager [C]. Every dictation door reads it for each request in the same way (DD-90), so an administrator's change applies at the next request, and each read of a bad file repeats its warning.

### Choke points

| Door | `enabled: false` | `capture: false` | Other keys |
|---|---|---|---|
| `POST /dictation/streams` | 403, MSG-POLICY | 403, MSG-POLICY-CAPTURE | the active model must be in `allowed_models` |
| `POST /dictation/models/install` | 403 | allowed | `model_downloads: false` refuses; `allowed_models` refuses others |
| `POST /dictation/models/select` | 403 | allowed | `allowed_models` refuses others |
| `POST /dictation/models/rescan`, `POST /dictation/bench` | 403 | allowed | under `model_downloads: false`, rescan moves nothing and only reports |
| `POST /dictation/models/delete`, dictionary export | never refused | never refused | never refused |
| `/dictionary/*` writes and observations | 403 | allowed | `dictionary: false` refuses |
| `add_extension_with_origin` | the `dictionary` capability does not load, so a resumed roster cannot load it | | `dictionary: false` likewise |
| `permission_for` | denies `dictionary__*` | | `dictionary: false`, `dictionary_agent_writes: false` |
| `call_tool_boundary_refusal` [C `routes/agent.rs:3320`] | a managed deny arm, which also closes the general gap that a managed deny does not hold at `POST /agent/call_tool` | | |
| The chat extractor | off | | `dictionary: false`, `dictionary_agent_writes: false` |
| `bias_set` [C `crates/biorouter/src/dictation/bias.rs`] | | | `dictionary: false` gives the empty set, so a dictation sends no prompt, builds no bonus table, corrects nothing and records no usage; `transcribe` then prepares no dictionary at all, as with `--no-dictionary` (PR-14b) |
| Every CLI door | exit 2 with MSG-POLICY | `dictate` and the terminal front ends exit 2 with MSG-POLICY-CAPTURE; `transcribe` works | `install` refuses under `model_downloads: false` unless its target is `models_dir` |
| Tray and global shortcut, Electron main for an external backend | removed | removed | |

The reason sentence for every refusal rides in the response and in status as an `Allowance`. The message catalog in [surfaces](surfaces.md) holds the four policy sentences: MSG-POLICY, MSG-POLICY-CAPTURE, MSG-POLICY-DOWNLOADS (a refusal under `model_downloads: false`; it names the managed `models_dir`) and MSG-POLICY-MODELS (a refusal under `allowed_models`; it names the allowed ids). The interface never learns policy from `GET /config`, which renders keys as editable rows; it reads `GET /dictation/status` `policy` only.

The switch is accident prevention, not a boundary. `BIOROUTER_PATH_ROOT` relocates the policy file [C `managed_policy_path`], and on Windows a person can create `%ProgramData%\Biorouter`. `docs/security/managed-policy.md` documents the block in PR-05.

### Operator keys (env only)

| Variable | Meaning |
|---|---|
| `BIOROUTER_DICTATION_MODELS_DIR` | An extra read only model folder, second in the lookup order |
| `BIOROUTER_DICTATION_BROWSER_INSTALL` | `false` locks browser installs on a keyless daemon; `biorouter serve --no-dictation-install` sets it |
| `BIOROUTER_DICTATION_MODEL_MIRROR` | A mirror base URL tried first, same hashes |
| `BIOROUTER_DICTATION_WORKER` | Absolute path of another worker binary; refused unless its `hello.protocol` matches |
| `BIOROUTER_DICTATION_RECORDER` | Linux only: a command that writes raw s16le 16 kHz mono to stdout, for the terminal ([surfaces](surfaces.md)) |
| `BIOROUTER_DICTATION_TEST_MODEL` | Debug builds only: the path of a vendored `for-tests` model, loaded on the CPU in place of a catalog model when no model is named, with the `for-tests` detector beside it; both must hash to their pins. It lets route and CLI tests run the whole path offline (the fixture decodes to nothing [M]); a release build ignores it |

They are read with `std::env::var`, never `Config::get_param`, so `POST /config/upsert` cannot set them (DD-91). The operator lock composes with managed policy as the most restrictive of the two.

### Configuration keys and why they are not classified

| Key in `config.yaml` | Meaning |
|---|---|
| `BIOROUTER_DICTATION_MODEL` | The active model id |
| `BIOROUTER_DICTATION_LANGUAGE` | `en` (default), `auto` or ISO 639-1; a `.en` model refuses other values |
| `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS` | Default 300 |
| `BIOROUTER_CLI_DICTATION_KEY` | The terminal's dictation key ([surfaces](surfaces.md)) |
| `BIOROUTER_CLI_DICTATION_DEVICE` | The terminal's input device ([surfaces](surfaces.md)) |
| `dictation_dictionary_activation_version` | The activation marker of DD-56, hidden in `ConfigSettings.tsx` |

The keys in this table stay out of both `CAPABILITY_CONFIG_KEYS` and `NOT_CAPABILITY_CONFIG_KEYS` in [`config_keys.rs`](../../crates/biorouter/src/privacy/config_keys.rs). `every_config_key_the_tier_resolver_reads_is_classified` (`:254`) fails any classified key that no tier input file reads, with `{key} is classified but no tier-input file reads it; delete its row` (`:317`) [C]. No dictation key is a tier input: the active model decides recognition quality, never which provider a chat binds.

## Daemon routes and transport

### Route table

`crates/biorouter-server/src/routes/dictation.rs` is new in PR-15, merged in `routes::configure` behind `check_token` beside the other route modules, and annotated with utoipa. After it lands, `just generate-openapi && cd ui/desktop && npm run generate-api` regenerates the client, and `scripts/check-openapi-schema.sh` must be clean.

| Route | Body | Answer | Authority |
|---|---|---|---|
| `GET /dictation/status` | none | the status shape below | secret |
| `POST /dictation/streams` | JSON at most 8 KiB `{sample_rate, format: "s16le", language?, target_kind: "chat" \| "new_chat" \| "editor" \| "crew" \| "external", session_id?, new_chat_provider?, new_chat_model?, include_private_terms?, context?}` (`new_chat_provider` is the per tab provider pick for `new_chat`, and `new_chat_model` the tab's model beside it, so a provider whose tier reads the model, such as an Ollama cloud tag, is judged as the chat will bind it; both honored only from a caller with the proof; `include_private_terms` only with `crew` or `external`, honored only from a caller with the proof and private standing; `context` at most 2 KiB, ranking only, never logged) | 201 `{stream_id, max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600}`; 409 `noModel`; 429 `tooManyStreams` with `Retry-After`; 403 with the policy sentence; 503 `serverMemory` | secret (O-17); `session_id` through `session_reach` first |
| `PUT /dictation/streams/{id}/audio?seq=N` | `application/octet-stream`, raw s16le, at most 524,288 B | 200 `{received_s, partial?, queue_position}`; 409 `outOfOrder {expected_seq}`; 410 expired or cancelled; 413; 500 `engineFailed` once a decode failed while the person spoke | secret |
| `POST /dictation/streams/{id}/finish[?seq=N]` | none, or the tail chunk as `application/octet-stream` with its `seq`, taken as a `PUT` would take it, so a stop costs one round trip | 200 transcript, or 202 `{pending: true}` after 3 s; 500 `engineFailed`; for the tail chunk the `PUT`'s 409, 413 and 422 | secret |
| `GET /dictation/streams/{id}[?wait_ms=N]` | none | as finish, held up to `wait_ms` (at most 3,000) until the transcript is ready, else answered at once | secret |
| `DELETE /dictation/streams/{id}` | none | 204; queued segments dropped, the audio and the transcript zeroed | secret |
| `POST /dictation/models/install` | `{model}` (catalog id) | 202, progress in status | keyed: proof; keyless: secret unless locked |
| `POST /dictation/models/cancel` | `{model}` | 200 | secret |
| `POST /dictation/models/delete` | `{model}` | 200; never refused by policy | keyed: proof; keyless: secret |
| `POST /dictation/models/select` | `{model?, language?}` | 200; writes `BIOROUTER_DICTATION_MODEL` and `BIOROUTER_DICTATION_LANGUAGE` | keyed: proof; keyless: secret |
| `POST /dictation/models/rescan` | none | 200 `{added: [id…], unrecognized: [file…]}`; files identified by SHA-256 against the catalog; 403 with the policy sentence under `enabled: false`; under `model_downloads: false` nothing is placed, and a catalog file that is not placed is not `unrecognized` | secret |
| `POST /dictation/bench` | none | 200 `{model, rtf, elapsed_ms}` | secret |

No route takes a filesystem path. There is no `/dictation/models/import` and no one shot upload route.

The transport is one chunked HTTP stream for every Biorouter page (DD-27). The desktop and `serve` build one renderer source with one Vite config, and `utils/surface.ts` tells them apart at run time [C `ui/desktop/package.json` `build:web`]. Uploads of 64 to 512 KiB took about 3 to 14 ms (2.7 to 14.1 ms in Chromium, 3 to 7 ms in Firefox and WebKit), and nginx's 1 MiB default answers a readable 413 [M]. A WebSocket would be a fourth entry in `auth.rs` `is_unauthenticated_path`, after `/status`, `/ui/workspace` and `/tool_bridge/{nonce}`, and the second exempt path after `/ui/workspace` to take the daemon secret as a query parameter, as `routes/workspace.rs` `check_workspace_ws_auth` and `hooks/useWorkspaceChannel.ts` do today [C]. It also failed through a stock nginx location [M]. The renderer's six connections per origin already hold up to five long requests by design (`MAX_LIVE_OBSERVER_STREAMS = 2` at `state.rs:879`, plus the long polls) [C], so dictation adds no long lived request.

### Request and response shapes

```ts
// The generated client's types, written out
type Allowance = { allowed: boolean; reason: string | null }; // reason is the sentence to show

type DictationStatus = {
  engine: {
    compiled: boolean;
    state: 'no_engine' | 'unsupported_cpu' | 'stopped' | 'starting' | 'ready' | 'error';
    backend: 'metal' | 'cpu' | null;
    device: string | null;
    threads: number;
    missing_cpu_features: string[];
    rss_bytes: number | null;
  };
  models: Array<{
    id: string; display_name: string; installed: boolean; bytes: number;
    location: 'user' | 'system' | 'managed' | 'env' | null;
    superseded: boolean;
    suitability: 'recommended' | 'suitable' | 'too_slow' | 'below_minimum' | 'unsupported_cpu' | 'unknown';
    recommended: boolean; // the class default
  }>;
  active_model: string | null;
  language: string;
  install: Allowance; // for this caller
  install_job: {
    model: string; bytes_done: number; bytes_total: number; bytes_per_second: number;
    phase: 'downloading' | 'verifying' | 'installed' | 'failed'; error: string | null;
  } | null;
  policy: {
    managed: boolean;
    enabled: Allowance; capture: Allowance; install: Allowance; external_engine: Allowance;
    dictionary: Allowance; agent_writes: Allowance;
    allowed_models: string[] | null;
  };
  limits: { max_seconds: number; max_chunk_bytes: number; max_streams: number }; // 600, 524288, 8
  queue: { streams: number; segments: number };
  account: string;
  host: string;
  models_dir: string;
};

type Transcript = {
  text: string;      // corrected
  raw_text: string;  // as the engine wrote it, after the nonspeech filter
  changes: Array<{ start: number; end: number; from: string; to: string; stage: string; entry_id?: string }>;
  segments: Array<{ t0: number; t1: number; text: string }>; // seconds
  model: string;
  audio_s: number;
  elapsed_ms: number;
  bias_scope: 'public' | 'private'; // public for any caller without private standing
  retried: null | { reason: string };
  no_speech: boolean;
  // Where the stop path's time went: segments decoded (the tail included); from `finish`
  // to the tail's decode starting (a segment still decoding, or a load still running);
  // the tail's decode; and the tail's length.
  timings: { segments: number; wait_ms: number; tail_ms: number; tail_audio_s: number };
};
```

The correction stages and retry reasons are defined in [dictionary](dictionary.md). `biorouter transcribe --format json` (schema 1) uses these names for every field the two shapes share (`text`, `raw_text`, `changes` with `stage`, `model`, `audio_s`, `bias_scope`) and adds `schema`, `language` and `timings`, so `transcribe`'s callers and the renderer read one vocabulary. No response reports how many dictionary entries were eligible or withheld (P-6). `policy.install` reports only what managed policy and the operator lock allow; the top level `install` adds the engine, the processor and this caller's authority.

`install.reason` for a caller, in order of precedence: MSG-NO-ENGINE when the engine is not compiled; MSG-POLICY when policy forbids dictation; the CPU sentence of [Hosts without AVX2](#hosts-without-avx2) when the host class is `NO`; MSG-POLICY-DOWNLOADS when downloads are forbidden; MSG-INSTALL-AGENT for a caller without the proof on a keyed daemon; MSG-INSTALL-LOCKED on a keyless daemon with the operator lock. So no interface offers a button that would fail.

### Errors

Every refusal answers `{code, message, expected_seq?}`, following the `message` field of the existing `ErrorResponse` [C `routes/errors.rs`]:

| Status | `code` | When | `message` |
|---|---|---|---|
| 403 | `policy` | managed policy forbids this door | MSG-POLICY, MSG-POLICY-CAPTURE, MSG-POLICY-DOWNLOADS or MSG-POLICY-MODELS |
| 403 | `proofRequired` | install, select or delete without the proof on a keyed daemon | MSG-INSTALL-AGENT |
| 403 | `installLocked` | install on a keyless daemon under the operator lock | MSG-INSTALL-LOCKED |
| 409 | `notCompiled` | a build without the engine | MSG-NO-ENGINE |
| 409 | `unsupportedCpu` | the host class is `NO` | the CPU sentence |
| 409 | `noModel` | no verified speech model or detector | this caller's `install.reason`, or empty when installing is allowed (the interface then opens the setup popover) |
| 409 | `languageUnsupported` | a `.en` model with another language | MSG-LANGUAGE-EN |
| 409 | `outOfOrder` | `seq` above the next expected value | empty; `expected_seq` set |
| 409 | `finished` | a new chunk after `finish` (a repeat of an acknowledged `seq` is still answered as before) | empty |
| 409 | `installBusy` | another model is installing | empty |
| 409 | `notDeletable` | the copy is in a system, managed or env folder | empty |
| 410 | `streamGone` | expired, cancelled, or delivered and dropped | empty |
| 413 | `tooLarge` | a chunk over 524,288 B, a JSON body over 8 KiB, a `context` over 2 KiB, or a chunk that would carry the stream past 600 s | empty |
| 422 | `badRequest` | an unknown id, a `format` other than `s16le`, a `sample_rate` outside 8,000 to 96,000, a malformed body, a chunk with an odd number of bytes, a `finish` body without its `seq` | empty |
| 429 | `tooManyStreams` | 8 open streams, or 32 queued segments; `Retry-After` set | MSG-SERVER-BUSY is the client's text |
| 500 | `engineFailed` | the worker exited, or missed its result deadline after the GPU to CPU retry; answered by `finish` and by a poll, and, when a segment's decode failed while the person still spoke, by every later `PUT`, so the client shows MSG-ENGINE-FAILED at once and the stream takes no more audio (its detector passes stop too) | MSG-ENGINE-FAILED |
| 503 | `serverMemory` | the memory check before a load fails | MSG-SERVER-MEMORY |

A proxy's own 413 for a small chunk is the client's MSG-413 ([surfaces](surfaces.md)); a conforming client never meets the daemon's own 413.

### Stream lifecycle

1. **Create.** The handler checks policy, the engine and the CPU class; validates the body; resolves the model and checks the memory; then, before anything else, calls `service().prepare(model)` without waiting for it, so no decision after it delays the load. It answers 201 without waiting for the target either: the target tier and its model's affiliation ([Target and bias decision](#target-and-bias-decision)) and the bias set are decided once, on a task of their own beside the load, and the decoder waits for them before its first decode. A slow decision (a new chat's first provider build) therefore holds neither the 201 nor the load, and the 201's timing says nothing about the target chat. The stream id is 128 random bits in hex. A `sample_rate` other than 16,000 (the native rate fallback for Firefox before 148: only Firefox 146.0.1 was measured [M], and the 148 boundary comes from Bugzilla 1674892 [R]) is resampled in the daemon by `audio.rs`.
2. **Append.** Each PUT carries the next `seq`, starting at 0, strictly increasing. A repeated `seq` is idempotent: the daemon answers as before and ignores the body. A higher `seq` answers 409 `outOfOrder` with `expected_seq`. The route sets `DefaultBodyLimit::max(524_288)`; the JSON routes allow 8 KiB.
3. **Segment.** Once 8 s or more are buffered past the decoded mark, the service asks the worker for `vad_segments` over the undecoded tail and queues each closed segment, split at the longest pause so none exceeds 25 s, decoded with `no_context = true` (DD-50). One detector pass reads at most 60 s (`MAX_DETECTOR_PASS_MS`): audio that arrives in real time never fills it, and audio that arrives at once (a replay after a 410, the buffer a client kept while creation answered 429) is read in passes of that size, because detector requests go ahead of every stream's decodes in the supervisor and one pass over minutes of audio held another person's stop behind it [M, `a_short_stream_finishes_while_a_long_one_runs`]. Every pass over 33 s or more moves the mark (`plan_pass` in the routes): a window with no speech moves it to 1 s before the window's end, so a long silence is read once; and speech with no pause the detector closes, once it runs 33 s past the mark, is cut every 25 s as the plan at stop cuts it, because synthetic voices end to end (a minute with no 100 ms pause) were cut nowhere while they streamed and left all 60 s to decode at stop: 1.14 s on `small.en` and 1.32 s on turbo, Metal [M, 2026-11-06]. While 32 segments are already queued, closed segments wait in their stream, so a PUT never answers 429.
4. **Partial text.** A PUT answer carries `partial`, the corrected text of the segments decoded so far, when it changed. The interface shows it only as a muted preview line, never in the field (DD-31).
5. **Finish.** `POST …/finish` carries the tail chunk with its `seq` (taken as a `PUT` would take it, so a stop costs one round trip; a resent `seq` is a repeat), queues the open tail and waits up to 3 s for the whole transcript, then answers 200, or 202 `{pending: true}`. After a 202 the client reads `GET /dictation/streams/{id}?wait_ms=3000`, which the daemon holds until the transcript is ready, so no client sleep sits between two polls. Once the person has stopped, no new closed segment is cut, and the chunk `finish` carries starts no detector pass. A tail of 25 s or less at 16 kHz goes straight to `transcribe`, whose integrated detector trims it; a longer one, and a whole recording at another rate, is planned like a recording. The tail's clock starts after the load started at creation has answered, so a model still loading at stop is reported in `timings.wait_ms`, not as decoding.
6. **Drop.** The audio buffer is a `zeroize` (1.8.2, already in `Cargo.lock`) buffer, zeroed when the transcript is ready, on `DELETE`, or at the 60 s idle expiry (P-1). Each queued segment and each detector pass holds its own copy of its audio, an `AudioCopy` that zeroes itself when it is dropped, so a stream deleted, expired or failed with segments still queued wipes every copy, not only the main buffer; the decoder drops its queue as it ends. The transcript is held until `DELETE`, which the client sends after it inserts the text, or until 60 s after it was last read; then its strings are zeroed and the stream answers 410. A read that crossed the `DELETE` answers 410 too.

The stream holds at most 600 s of audio (19.2 MB at 16 kHz). A PUT that would pass it answers 413, and the audio already received is still transcribed on `finish`. Audio is never written to disk, a log, a diagnostics bundle or a session row (P-1, DD-93).

### Target and bias decision

The daemon decides once per stream, never the client (DD-30, P-4), on a task the creation spawns after it started the model load and before it answers 201:

```rust
// crates/biorouter-server/src/routes/dictation.rs, `stream_dictionary` and `stream_target`
let caller = session_reach::http_caller(&headers).await;
// the proof or a private provider stated in X-Caller-Provider; never served_operator (SD-10)
let standing = caller.dictation_standing();
let enforced = biorouter::privacy::privacy_tiers_enabled(); // loaded once per process
let proven = auth::user_action_proof(&headers) == UserActionProof::Proven;

// The target's tier and its model's affiliation come from one provider read (DD-30, O-32);
// None is an unstated affiliation, which admits only private rows with no institution.
let public = || BiasTarget { private: false, affiliation: None, person_included: false };
let target = match (req.target_kind, req.session_id.as_deref()) {
    (TargetKind::Chat | TargetKind::Editor, Some(id)) => {
        // resolve the tier before touching the session; a refusal degrades to public, silently
        match session_reach::session_reach(&manager, id, &headers).await {
            Ok(()) => {
                let stored = match session_reach::target_tier(&manager, id).await {
                    TargetTier::Private => Some(SessionClassification::Private),
                    TargetTier::Public => Some(SessionClassification::Public),
                    TargetTier::Unreadable => None,
                };
                // one read of the live agent's provider: its tier() and affiliation(); a
                // provider Gate B' refuses (public on a private row) reads as no live model.
                // `live_model` skips a chat a Crew grant scopes and gives up after 250 ms
                // (LIVE_MODEL_BOUND); either way the chat is judged by its row alone.
                let live = match state.peek_agent(id).await {
                    Some(agent) => live_model(&agent, id).await, // Some((tier, affiliation))
                    None => None,
                };
                BiasTarget {
                    private: target::chat_reach(stored, live.map(|(tier, _)| tier)) == ChatReach::Private, // Unknown counts as public
                    affiliation: live.and_then(|(_, affiliation)| affiliation), // unstated when no agent is live
                    person_included: false,
                }
            }
            Err(_) => public(),
        }
    }
    (TargetKind::NewChat, _) => {
        // DD-30: the tier the new chat's bind would produce; the per tab pick counts only with the proof
        let pick = req.new_chat_provider.as_deref().filter(|_| proven);
        let model = req.new_chat_model.as_deref().filter(|_| proven);
        let (tier, affiliation) = target::new_chat_bind_tier(pick, model).await; // pick, else BIOROUTER_PROVIDER; Public on failure
        BiasTarget { private: tier.is_private(), affiliation, person_included: false }
    }
    (TargetKind::Crew | TargetKind::External, _) => {
        // O-8: only for one recording the person confirmed after MSG-PRIVATE-TERMS; every private row then passes, institution tagged rows included (O-37)
        let included = req.include_private_terms && proven && standing.is_private();
        BiasTarget { private: included, affiliation: None, person_included: included }
    }
    _ => public(), // chat or editor without a session id
};
let bias = biorouter::dictation::bias::bias_set(&view, &target, standing, enforced);
```

`target_tier` [C `session_reach.rs:676`] reads metadata only; `peek_agent` [C `state.rs:1967`] catches a live chat whose bound provider is private before its row says so. The live model is read through `Agent::provider`, the only public accessor, which for a chat a Crew grant scopes asks the Crew workspace over SSH (`context.manifest`) when the grant's last admission is older than 15 s, and forgets that admission first [C `crates/biorouter/src/crew/mod.rs` `check_provider_use`]. So the route never reads it for such a chat (`is_scoped_session`, a registry lookup), and gives up on any other after 250 ms; either way the chat is judged by its stored tier alone with an unstated affiliation, which fails toward public terms. A side effect free accessor on `Agent` for the bound provider's tier and affiliation would let a Crew chat's live model count too; it does not exist yet [C `agents/agent.rs`, where the provider field is `pub(super)`]. `chat_reach` in `crates/biorouter/src/dictation/target.rs` is the one function that answers "is this chat private?" for the stream target, the observation stamp and the extractor stamp ([dictionary](dictionary.md)); a stream target counts its `Unknown` as public. A refusal from `session_reach` [C `session_reach.rs:695`] gives public scope and a byte identical response, so the route answers no question about another chat. `bias_set` and its truth table are in [dictionary](dictionary.md); with the privacy master switch off every entry is eligible. Private rows carry the institutions of the chat they were written in, those behind its model and those whose connectors it touched, decided by the owner on 2026-11-05 (O-32, O-35), and `bias_set` keeps such a row only when every institution it names admits the target's model, each asked through `affiliation::gate_cross_affiliation_warning` [C `crates/biorouter/src/privacy/affiliation.rs`] ([private rows and institutions](dictionary.md#private-rows-and-institutions), P-16). The target side is the target's model alone: a chat's touched institutions describe what that chat holds, and no reader in the code adds them to the model side of a check [C `crates/biorouter/src/privacy/capability.rs` `CallCapability::cross_affiliation_warning`, `crates/biorouter/src/session/chat_history_search.rs` `affiliation_clause`], so this route reads none for the target. The target's model affiliation comes from the same provider read as its tier: the live agent's bound provider for a chat, unstated when no agent is live, and the instance `new_chat_bind_tier` builds for a new chat [I]. A row left out changes no field of the response. `new_chat_provider` carries the composer's per tab provider pick for Home and a new tab. From a caller without the proof it is ignored and `BIOROUTER_PROVIDER` decides, so an unproven caller cannot choose the tier its own stream is judged by (P-4).

`new_chat_bind_tier` judges a new chat by the tier its bind would produce, never by a declared tier, because a declared tier is permissive in exactly the cases that matter here: `ollama` declares Private and resolves Public when `OLLAMA_HOST` points off the machine [C `session_reach.rs` `caller_capability`], its instance `tier()` is Public for a cloud tag such as `gpt-oss:120b-cloud` [C `providers/ollama.rs` `tier`], `llamacpp` with a remote `LLAMACPP_EXTERNAL_HOST` is Public the same way [C `providers/llamacpp.rs` `tier`], and a lead and worker pair binds `ProviderTier::least` of the two. So the function builds the provider from the configuration (with the pick in place of `BIOROUTER_PROVIDER` when there is one), takes its instance `tier()`, reduces a lead and worker pair with `ProviderTier::least`, and answers Public on any failure (DD-30). It caches the answer for 10 s (`NEW_CHAT_CACHE_TTL`), keyed by the pick, the model and the value of every key a tier input file reads: `CAPABILITY_CONFIG_KEYS` and `NOT_CAPABILITY_CONFIG_KEYS` both, because the second list holds tier inputs too (`VERSA_AZURE_ENDPOINT` and `VERSA_BEDROCK_ENDPOINT` move a Versa provider off the UCSF gateway, which makes it Public with no affiliation [C `privacy/config_keys.rs`]). The lifetime covers what no configuration value carries: `ollama`'s `tier()` also reads what `/api/show` reported about the model (`remote_host` or `remote_model`, a cloud model renamed with `ollama cp`) [C `providers/ollama.rs` `remote_origin`], which a first answer may predate. It returns the instance's `affiliation()` beside the tier, and a lead and worker pair already folds both halves there [C `providers/lead_worker.rs` `affiliation`, `providers/mod.rs` `composite_affiliation`]; on a failure the affiliation is unstated. The private draft flag judges "a public model" the same way, by the daemon's instance tier for the target chat, and warns if dictated text then goes to one ([surfaces](surfaces.md)).

Standing is `HttpCaller::dictation_standing`, a new function inside `crates/biorouter-server/src/routes/session_reach.rs` with its own census row, because `HttpCaller::capability` (`:966`) and `admits` (`:1051`) are private there [C]. It reads two inputs: the proof, and a private provider stated in `X-Caller-Provider` (`CALLER_PROVIDER_HEADER`, `:315`, read at `:661` by `caller_capability`). It never reads the third input `HttpCaller` carries, a `serve` daemon's operator tier for a request from its own page (`served_operator`): SD-10 gives the served interface its operator's reach on listings and knowledge bases and nothing else, and `HttpCaller`'s own documentation keeps it out of every other decision [C]. Listing private rows (`lists_dictionary_entry`) follows SD-10 like any other listing; biasing and stamping do not. A `serve` browser on a private host keeps private standing anyway, because `userActionHeaders()` states the host's provider in `X-Caller-Provider` on a browser surface (SD-12) [C `ui/desktop/src/utils/userAction.ts`].

### Server limits

| Limit | Value (DD-28, DD-32) |
|---|---|
| Open streams per daemon | 8; the ninth answers 429 `tooManyStreams` with `Retry-After` |
| Queued segments | 32; stream creation then answers 429 |
| Chunk | 524,288 B |
| JSON body, `context` | 8 KiB, 2 KiB |
| Audio per stream | 600 s (the client warns at 570 s) |
| Idle expiry | 60 s |
| `finish` wait | 3 s, then 202; a read after it is held up to 3 s |
| Detector pass while streaming | at most 60 s of audio |
| Memory before a load | Linux `MemAvailable` and the cgroup's `memory.max` minus `memory.current` against the model's file size plus 512 MiB [I, calibrated on the measured peaks of 1.06 GB and 0.60 to 0.72 GB]; a shortfall answers 503 with MSG-SERVER-MEMORY. Skipped while the worker holds a model. Other systems are not checked, and the RAM floors of suitability apply instead; a debug build reads `BIOROUTER_DICTATION_TEST_FREE_MEMORY` in place of both figures on every system, for the route tests |
| Threads | `min(8, available_parallelism)`, halved while the llama sidecar or a loopback Ollama serves |

The daemon relies on the client's side of DD-28, which [surfaces](surfaces.md) specifies: worklet chunks of 1,600 frames (100 ms) buffered in memory; a PUT every 2 to 3 s with exactly one in flight and a 10 s timeout, the same `seq` resent after a timeout; a 413 or a network error on a chunk over 64 KiB halves the chunk, with a floor of 64 KiB; after the stop, the remainder rides on `finish` when it fits in one chunk; after a 202, reads the daemon holds up to 3 s, one after another with no sleep between (PR-15, replacing DD-28's 500 ms polls); MSG-WAIT after 1.5 s without an answer. Uploads of 64 to 512 KiB took about 3 to 14 ms; over HTTP/2 with ten streams held, a PUT took 7 to 17 ms [M].

On a hub with one daemon per person, each active dictating account loads its own worker (0.6 to 0.9 GB, freed after 300 s idle); whisper.cpp has no memory mapped load path, so the memory cannot be shared [R]. One daemon per operator, as `serve` is today, serializes the decodes of everyone it serves on one worker.

### Authority and O-17

Every `/dictation/*` route takes the daemon secret through `check_token` (DD-29). Model install, select and delete need the user action proof on a daemon that holds a user action key (DD-20). Streams take the secret alone, decided by the owner on 2026-11-05 (O-17): on a keyed daemon the proof and private standing decide only which entries may bias. S17 against a keyed server without its key therefore dictates with public terms only and shows MSG-REMOTE-NO-KEY; `tests/dictation_keyed.rs` asserts that streams are admitted on the secret.

Transcription changes no tier and sends nothing off the host, and a secret holder that states a private `X-Caller-Provider` can already list private rows [C `session_reach.rs` `HttpCaller::lists_session` at `:982`, which reads `CALLER_PROVIDER_HEADER` (`:315`) through `caller_capability` at `:661`], so requiring the proof for streams would add little at the cost of S17.

### Privacy census

`crates/biorouter/tests/privacy_guard_wiring.rs` gains `crates/biorouter-server/src/routes/dictation.rs` as a site on the existing rows `http_caller`, `session_reach` and `target_tier` (whose row moves from `Status::WiredThrough("session_reach")` to `Status::Wired`), a `Guard` row for `HttpCaller::dictation_standing` (`defined_in: SESSION_REACH`), and rows written by hand for `bias_set` and `chat_reach`, as [dictionary](dictionary.md) lists; the `every_gated_route_resolves_the_tier_before_it_touches_the_session` table in `session_reach.rs` gains the stream creation handler. PR-14 adds the `visible_to` site in `crates/biorouter/src/dictation/bias.rs`. PR-14 also adds a `Site` row to `crates/biorouter/tests/privacy_capability.rs` for the one `affiliation::gate_cross_affiliation` call in `bias.rs`, the institutions check of private rows, which sits inside the loop over a row's institutions (P-16, O-35). No pull request here adds a `floor`, `raise_privacy`, `.call_tool(` or `CallCapability::sample(` site.

### Logging

Tracing in the dictation routes and the service records a stream's log id, `target_kind`, `bias_scope`, durations, byte counts, queue positions and model ids, nothing else (DD-93). The log id is the first 8 hex characters of the stream id's SHA-256, never the id itself: the id is the only thing besides the daemon secret that the stream's routes check (DD-29), so a log reader who holds the secret could otherwise read a private chat's transcript with it. It never records audio, `text`, `raw_text`, `partial`, `changes`, prompt strings, dictionary entries or `context`. `tests/dictation_routes.rs` captures tracing output and fails on any audio derived content (P-1, P-2).

**Acceptance (PR-15).** The route binaries of [testing and release](testing-and-release.md) pass: `dictation_routes`, `dictation_routes_auth`, `dictation_keyed`, `dictation_no_user_key` (its own binary, because the user action digest is a process global `OnceLock`), `dictation_policy_routes`, and the oracle case in `dictation_routes` (a secret only caller, near silence and a private chat's id get public scope and a byte identical response). The OpenAPI schema check is clean and the TypeScript client compiles.

## Removal of the inherited cloud dictation

The inherited cloud dictation is a daemon route that forwards audio to OpenAI or ElevenLabs on the daemon secret alone. It is deleted in the first pull request, PR-01 (DD-89, O-10, decided by the owner on 2026-11-05). While it is merged, "audio is not sent anywhere else" is false, so no document may say that of a build before PR-01.

What is deleted:

| Item | At `main` |
|---|---|
| `crates/biorouter-server/src/routes/audio.rs` | `POST /audio/transcribe` (OpenAI `whisper-1`), `POST /audio/transcribe/elevenlabs` and `GET /audio/config` [C] |
| Two lines of `routes/mod.rs` | `pub mod audio;` (`:1015`) and `.merge(audio::routes(state.clone()))` (`:1069`) [C] |
| `ui/desktop/src/hooks/useWhisper.ts`, `useDictationSettings.ts`, `dictationConstants.ts` | Nothing outside them and the settings folder imports them [C] |
| `ui/desktop/src/components/settings/dictation/` | Six files, mounted by nothing: `DictationSection.tsx`, `ElevenLabsKeyInput.tsx`, `ProviderInfo.tsx`, `ProviderSelector.test.tsx`, `ProviderSelector.tsx`, `VoiceDictationToggle.tsx` [C] |
| `VOICE_DICTATION_ELEVENLABS_ENABLED` | `ui/desktop/src/updates.ts:5` [C] |
| The exclusions for the folder | `settingsVocabulary.test.ts`, `emphasisWeight.sourceGuard.test.ts` and `typeScale.test.ts` exclude `settings/dictation/` [C]; the exclusion goes with the folder |

The route was never in the OpenAPI document [C], so the generated client does not change. The renderer clears `localStorage['dictation_settings']` once. Saved `ELEVENLABS_API_KEY` entries in the credential store are left alone, and the release notes say so; `OPENAI_API_KEY` is the OpenAI provider's own key and is not touched.

Documentation reworded in PR-01: the "Voice dictation (Whisper)" line of `docs/cli/qa-checklist.md`, which lists voice dictation as a desktop only feature [C], becomes a pointer to `docs/dictation/testing-and-release.md` (PR-33 retargets it to `docs/releases/dictation-test-checklist.md`); `docs/desktop-ui/settings-visual-vocabulary.md` and `docs/desktop-ui/shared-controls.md` stop describing the folder; the freeze lines in `docs/design/codex-simplicity-redesign/implementation-spec.md` are coordinated with the unmerged `feat/folder-access-hover-card` branch, which also edits that file.

Tests: a new `crates/biorouter-server/tests/no_cloud_audio_routes.rs` asserts that the three paths answer 404 through `routes::configure` and `check_token`, and a source scan fails on `api.openai.com/v1/audio` or `elevenlabs.io` under `crates/` and `ui/desktop/src`.

**Acceptance (PR-01).** `git grep -n "useWhisper\|VOICE_DICTATION_ELEVENLABS\|audio::routes"` is empty; `cargo test -p biorouter-server` and `cd ui/desktop && npm run test:run && npm run lint:check` pass; `scripts/check-openapi-schema.sh` shows no diff.

## What the build spike measured

On 2026-11-04 the design built whisper.cpp at the pin in Biorouter's own cross images, linked it into a small Rust binary with the same `cmake` 0.1.58 and `cc` 1.2.64 that `Cargo.lock` holds, and ran a self starting worker on every shipped target. The host was an Apple M4 Max on macOS 26.7.1 shared with other benchmarks, so absolute times are upper bounds; the yes or no results and the ratios hold. This document relies on these facts [M]:

| Area | Result |
|---|---|
| Linux, cross built in `rust:1.92-bullseye` | Correct JFK transcript with the real `base.en` model on Debian 11, Rocky 9.3 and Debian 12 (amd64 under Rosetta), 1.12 to 1.16 s through the worker; `DT_NEEDED` only `libgcc_s`, `libpthread`, `libm`, `libdl`, `libc` and the loader, all in `BASE_LIBS`; highest symbol `GLIBC_2.30`; no `GLIBCXX` |
| Windows, mingw in `rust:latest` (GCC 14) | Imports only system DLLs; correct transcript under wine 10 in 2.65 s; wine 8 cannot start a rustc 1.98 binary at all; GCC 14's win32 thread model runs `std::thread`; never run on native Windows (M-1) |
| macOS arm64 and x86_64 | The worker transcribed on Metal (`MTL0`) and, on x86_64, under Rosetta once `ROSETTA_ADVERTISE_AVX=1` was set |
| The macOS 15 symbol trap | With `MACOSX_DEPLOYMENT_TARGET` unset, the C and C++ objects were built for the SDK (27.2) while the binary said 11.0, and it strongly imported `MTLResidencySetDescriptor` (macOS 15), plus `cblas_sgemm$NEWLAPACK$ILP64` (macOS 13.3) with BLAS on. At 12.0 with BLAS off: `minos 12.0`, a weak import, no BLAS symbol |
| Old processors | Under `qemu-x86_64 -cpu Nehalem`, the engine linked test binary's `--version` exited 0, the static engine died with `SIGILL` (132), and the gated parent printed the CPU sentence and exited 20. Measured on the test binary, not on `biorouterd`, which is why DD-12 adds the check |
| Rosetta | CPUID hides AVX, AVX2, FMA, F16C and BMI2 under Rosetta; `ROSETTA_ADVERTISE_AVX=1` makes them visible and the worker correct |
| ISA levels | `x86-64-v3` was about 3.3 times faster than SSE4.2 and 6.4 times faster than SSE2 on the same clip (relative figures, under Rosetta) |
| The runtime dispatch helper | Does not build unchanged on bullseye's GCC 10.2 (two variants need GCC 11); after dropping them it ran and picked `haswell`, or `sse42` on Nehalem, at 17.6 MB against 2.2 MB |
| Docker | The Dockerfile's builder fails without `cmake`; with it the engine builds in 15 s and transcribes in `debian:bookworm-slim` on arm64 |
| Discovery | The Llama Server lookup (`find_binary`) builds its candidates from an unresolved `current_exe()` [C `llamacpp_sidecar.rs` `find_binary`]. On macOS that is the launch path, so a CLI reached through the `setup-path` symbolic link misses the bundled server and falls back to `PATH` [M]; on Linux `/proc/self/exe` is already resolved, and on Windows the CLI is a copy with nothing beside it [C `crates/biorouter-cli/src/commands/exe_path.rs`]. A worker that is the running binary needs no lookup |
| Build times | whisper.cpp static: 27 s Linux, 36 to 39 s Windows, 30 s macOS arm64; the Rust engine crate 19 s, 22 s and 75 s |
| Growth | +2.17 MB Linux, +3.84 MB Windows, +6.69 MB macOS arm64, +1.69 MB macOS x86_64 per binary |

A follow up on the same day measured the worker lifecycle on Metal, the hardened runtime, the MSVC prerequisites and the effect of `.cargo/config.toml` on both architectures; those results are in [The worker](#the-worker), [Metal](#metal) and [The MSVC test leg](#the-msvc-test-leg). The worklet inlining trap of the same period belongs to capture and is in [surfaces](surfaces.md).

## Related documentation

- [Implementation plan](implementation-plan.md): the requirements, the decision register this document elaborates, the pull requests and the owner decisions.
- [Dictionary](dictionary.md): the bias set, the token prompt, the decoder bonus and correction that the worker and the routes carry.
- [Surfaces](surfaces.md): capture, the client side of the stream, the setup popover, Settings and the full message catalog.
- [Testing and release](testing-and-release.md): every test named here, the CI jobs, the release guards and the measurements still to take.
- [Speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md): the measurements behind the engine choice, the catalog and the tiers.
- [Dictation tools survey](../research/voice-dictation/dictation-tools-survey.md): how other dictation apps package engines and models, and what they ran into.
- [Llama Server model catalog](../providers/llama-server/model-catalog.md): the compiled catalog precedent that `STT_MODEL_CATALOG` follows, and differs from on hashing.
- [Managed policy](../security/managed-policy.md): the policy file the `dictation:` block extends.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules behind the target and bias decision.
- [Linux service deployment](../deployment/headless-linux.md): the systemd unit and the `biorouter` service account that S8 installs models for.
- [Serve decisions](../deployment/serve-decisions.md): the keyless daemon rules behind O-2 and the SD-16 record.
- [Releases](../releases/README.md): the release process whose asset count and smokes the engine must not change.
