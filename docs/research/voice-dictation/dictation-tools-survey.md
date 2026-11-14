# Dictation tools survey

> **What this is.** A survey of how other dictation applications and speech engines capture audio, run speech models on the local machine, apply a personal vocabulary and distribute their models, written so Biorouter's local dictation design can learn from them.
> **Status:** Current: a snapshot of external projects as of 2026-11-04
> **Audience:** developers designing or reviewing Biorouter's local dictation: the engine, the model catalog, the composer button, and the dictionary with its agent tool.

Biorouter is adding local voice dictation: a microphone button in the chat composer, a speech model that runs on the person's own machine and is installed only when they ask for it, and a personal dictionary of terms such as UCSF and EPAS1 that grows with their work. Many projects already solve parts of this. This survey reads them at pinned commits and records what each one does, what it costs, and which parts a design like Biorouter's should copy or avoid. It covers four groups: speech engines and libraries, dictation applications, speech models other than Whisper, and the vocabulary features of commercial products. It ends with a comparison table and the lessons that recur across projects. Biorouter's own measurements are in [speech model benchmarks](speech-model-benchmarks.md), and its design is in [voice dictation](../../dictation/README.md).

## How to read this survey

Every load bearing claim carries one of four labels.

| Label | Meaning |
|---|---|
| **[code]** | Read in the project's source at the commit listed in the version table. |
| **[docs]** | Stated by the project, a vendor or a paper, and not reproduced by Biorouter. |
| **[measured]** | Measured by Biorouter's dictation research on 2026-11-04. Most of these runs used synthetic speech, so the relative effects matter more than the absolute numbers. |
| **[inferred]** | Reasoning from the facts above, not tested. |

Terms used throughout:

| Term | Meaning |
|---|---|
| STT | Speech to text. |
| VAD | Voice activity detection: a small model that marks which parts of the audio contain speech. |
| WER | Word error rate. Lower is better. |
| xRT or RTFx | Seconds of audio processed per second of compute. At 10×, a 10 s clip takes 1 s. |
| BPE | Byte pair encoding, the subword tokenizer Whisper uses. |
| ggml, GGUF | ggml is the tensor library under whisper.cpp and llama.cpp. GGUF is its current model file format. whisper.cpp still loads an older ggml `.bin` format. |
| Prompt biasing | Giving a decoder text, such as a glossary, that it treats as the transcript that came before. |
| Hotwords, shallow fusion | Adding a bonus during decoding to tokens that continue a listed phrase. |
| Transducer, CTC, AED | Three decoder families. Transducers include RNNT and TDT (token and duration transducer, used by Parakeet). CTC is connectionist temporal classification. AED is attention encoder decoder, used by Whisper and Canary. |

### Projects and versions examined

| Project | Kind | Version read | Date | License |
|---|---|---|---|---|
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp/tree/4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad) | C and C++ engine on ggml | master `4da27c41`; latest release v1.9.5 | 2026-11-03; release 2026-10-06 | MIT |
| [whisper-rs](https://codeberg.org/tazz4843/whisper-rs) | Rust bindings for whisper.cpp | crate 0.16.0, `whisper-rs-sys` 0.15.0 | 2026-03-12 | Unlicense |
| [faster-whisper](https://github.com/SYSTRAN/faster-whisper/tree/2a3a857737d17d974f30d818ebec69ce11929e5c) | Python engine on CTranslate2 | 1.2.1, `2a3a8577` | release 2025-10-31 | MIT |
| [whisper_streaming](https://github.com/ufal/whisper_streaming/tree/6da90b44b7e50d79695e68166d2a2c7609c75abb) | Python streaming policy | `6da90b44` | 2025-11-12 | MIT |
| [WhisperLive](https://github.com/collabora/WhisperLive/tree/740cc26669bc711f3a4d544ce3246dd02099c3b7) | Python WebSocket server and clients | 0.10.0, `740cc266` | release 2026-09-07 | MIT |
| [WhisperKit](https://github.com/argmaxinc/WhisperKit/tree/9f99b5cb6a907730543823ad271b82532782a899) | Swift framework on Core ML | `9f99b5cb` | 2026-10-10 | MIT |
| [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/tree/8c092a193f7878d827a26068e18f587576e9888d) | C++ engine on ONNX Runtime | 2.0.1, `8c092a19` | release 2026-10-26 | Apache-2.0 |
| [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp/tree/5700f127e38dd53c4735ea23ef743b4b538eacd4) | C and C++ engine on ggml, GGUF models | release 0.4.1; clone `5700f127` | 2026-10-22; 2026-10-27 | MIT |
| [Moonshine Voice](https://github.com/moonshine-ai/moonshine/tree/234f60faa0eb388b01cdf7e60aca232af37aefda) | C++ library on ONNX Runtime, own models | v0.1.5, `234f60fa` | 2026-08-24 | MIT |
| [OpenAI Codex](https://github.com/openai/codex/tree/6744e5eb79ec23b96cdfacf713b348a74f410702) | Coding agent; open CLI and TUI, closed desktop app | `6744e5eb` | 2026-11-04 | Apache-2.0 (open repository) |
| [Handy](https://github.com/cjpais/Handy/tree/82dfab9fda18c4bc24e1f6e56d36474d38eca0bc) | Desktop dictation app on Tauri | 0.10.0, `82dfab9f` | 2026-11-02 | MIT |
| [VoiceInk](https://github.com/Beingpax/VoiceInk/tree/5c95ff971687f71655a26b0c531833ac4414011b) | macOS dictation app in Swift | 2.25, `5c95ff97` | 2026-11-04 | GPL-3.0 |
| [OpenWhispr](https://github.com/OpenWhispr/openwhispr/tree/d5821a7c88f4c1803b06c51b4580f16c51dd0dbe) | Desktop dictation app on Electron | 1.11.0, `d5821a7c` | release 2026-10-29 | MIT |
| [Whispering](https://github.com/EpicenterHQ/epicenter/releases/tag/v7.11.0) | Desktop dictation app on Tauri (Epicenter) | v7.11.0, the last release with STT engines in the app; HEAD 7.11.1 `7cc869b4` | 2025-12-27; 2026-10-17 | AGPL-3.0 |
| [whisper-writer](https://github.com/savbell/whisper-writer/tree/370333b115417b03bf51c9f5bffd6578ecf6986e) | Desktop dictation app in Python | `370333b1` | 2024-08-24 | GPL-3.0 |
| [Buzz](https://github.com/chidiwilliams/buzz/tree/de7484bea317ebb17dcb27da3a6e4109f9bb33e7) | Desktop transcription app on PyQt | `de7484be` | 2026-11-04 | MIT |

The speech models in [Speech models beyond Whisper](#speech-models-beyond-whisper) carry their own dates and licenses.

## Speech engines and libraries

### whisper.cpp

**What it is.** A C and C++ implementation of OpenAI's Whisper on ggml, maintained by ggml-org. The latest release is [v1.9.5](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.5) (2026-10-06, tag commit `d1be6fde`). Master is 258 commits ahead, almost all ggml syncs, with a VAD state fix and a vocabulary check [code]. The tree now also builds NVIDIA Parakeet TDT through a separate `parakeet.h` API and `parakeet-cli` (PR #3735, 2026-06-16) [code]. Models use the legacy ggml `.bin` format, not GGUF [code].

**License.** MIT for the code. OpenAI states that Whisper's code and weights are MIT, and the ggml conversions and the Silero VAD model are MIT too [docs].

#### Library API

- The API separates an immutable `whisper_context` (weights and vocabulary) from a mutable `whisper_state` (caches, buffers, results). Several states can run on one context at once, one state per concurrent transcription. Functions without a `_with_state` suffix use the context's default state and are not safe to call from two threads [code].
- Input is 16 kHz mono 32 bit float PCM, at least 100 ms long. The library has no WebM or Opus decoder [code].
- The encoder always processes a padded 30 s window unless the experimental `audio_ctx` is set, so encoder cost is roughly constant per call whatever the utterance length. That cost dominates the latency of short dictation [code].
- `abort_callback` is checked after each encoder and decoder graph, so cancel takes effect within one encoder pass or one decode step [code].

The `whisper_full_params` fields that matter for dictation [code]:

| Field | Default | Behavior | Dictation note |
|---|---|---|---|
| `language` | `"en"` | `"auto"` or empty runs detection, which costs an extra encoder pass | Set it explicitly |
| `no_timestamps`, `single_segment` | false, false | Mask timestamp tokens; end after one segment | Useful for chunks of 30 s or less. On longer input, `no_timestamps` advances a full window and drops whole sentences at window boundaries (OpenWhispr's finding, below, which Biorouter's benchmark reproduced on 70 s clips [measured]) |
| `no_context` | true | Clears previous text history at the start of each call | Keep true so dictations do not bleed into each other |
| `initial_prompt`, `prompt_tokens` | null | Text placed after `<\|startofprev\|>`; `prompt_tokens` wins over `initial_prompt` | The dictionary glossary goes here |
| `carry_initial_prompt` | false | Keeps the prompt as a static prefix in every 30 s window (PR #3395, 2025-10-10; server field in #3781, 2026-05-15) | Matters only for input longer than one window |
| `n_max_text_ctx` | 16384 | Prompt history budget. `0` also disables the initial prompt (open issue #4098) | Never set it to 0 with a dictionary |
| `temperature`, `temperature_inc` | 0.0, 0.2 | Fallback ladder 0, 0.2, 0.4 and up to 1.0 on decoder failure | The prompt is dropped at 0.5 and above |
| `entropy_thold`, `logprob_thold`, `no_speech_thold` | 2.4, -1.0, 0.6 | Failure and silence detection | The entropy check runs only past 32 tokens. A segment is dropped as silence only when the no speech probability is high and the average log probability is low, so a confident hallucination passes |
| `suppress_blank` | true | Forbids end of text and a space as the first token | On silence, the first token must be text |
| `suppress_nst` | false | Suppresses a fixed list of punctuation and symbol tokens | Too broad for technical text: it stops `:`, `*`, `"`, `/`, `#`, `+`, `@` and `=` |
| `suppress_regex` | null | Regex matched against every vocabulary entry on every decoder step | Suppress only, and costly |
| `logits_filter_callback` | null | Host callback that may edit the logit row | The hook for dictionary boosting |
| `grammar_rules` | null | GBNF constraint with a soft penalty of 100 | For command vocabularies, not open dictation |
| `vad`, `vad_model_path`, `vad_params` | false | Built in Silero VAD, see below | Run VAD explicitly before decoding |
| `strategy` | caller | Greedy or beam | Beam search here draws candidates from a seeded distribution, not a classic top k. Greedy at temperature 0 is the predictable choice |

#### The prompt budget

The prompt budget is `min(n_max_text_ctx, n_text_ctx/2)` = 224 slots for every Whisper size. One slot holds `<|startofprev|>`, which leaves **223 usable tokens** [code]. When the prompt is longer, whisper.cpp keeps the **tail** and drops the front silently [code]. OpenAI's own prompting guide says the same: only the final 224 tokens count [docs, [Whisper prompting guide](https://developers.openai.com/cookbook/examples/whisper_prompting_guide)].

The prompt disappears without an error in four cases [code]:

| Condition | Effect |
|---|---|
| `n_max_text_ctx == 0` | No prompt at all |
| Temperature fallback reaches 0.5 or more | Prompt and history are omitted for that window |
| Multiwindow input whose final window is under 5 s | Prompt and history are cleared |
| `prompt_tokens` is set | `initial_prompt` is ignored |

Without `carry_initial_prompt`, the initial prompt becomes rolling history, and on input longer than 30 s the model's own output pushes it out.

#### Biasing hooks, grammars and suppression

- **`logits_filter_callback`** receives the mutable logit row and the tokens decoded so far in the current window [code]. It runs after temperature scaling and the built in special token suppression, and before `suppress_regex`, `suppress_nst`, the timestamp rules and the grammar. It is **not** removed at high temperature, unlike the prompt. It can run on several threads at once, one per decoder, during beam search or temperature fallback, so it must be reentrant [code]. Probabilities are computed after it runs, so a boosted token reports a boosted confidence [code].
- whisper.cpp has **no** hotword feature. Issue #1979 asked for one and was pointed at grammars, which the reporter found too slow for many words [docs].
- **Tokenizer pitfall.** `whisper_tokenize` takes the longest matching vocabulary entry greedily. It is not BPE. For 7 of 30 biomedical spellings it disagrees with Whisper's canonical BPE: `" EPAS1"` splits into `" EP"`, `"AS"`, `"1"` in BPE and into `" EPA"`, `"S"`, `"1"` in whisper.cpp [measured, using Whisper's own BPE file]. A bias trie built from `whisper_tokenize` ids misses what the model emits. Matching on bytes avoids the problem, and canonical BPE can be rebuilt because the converter writes tokens in merge rank order [inferred].
- **Grammars.** The GBNF parser lives in `examples/`, not in the library. A grammar penalizes every token that does not match and scans all 51,865 vocabulary entries per step. It constrains output rather than biasing it [code].
- **`suppress_regex`** builds a `std::regex` and runs it over the whole vocabulary on every decoder step. It can only suppress [code].

Prior art for dictionary boosting through the callback: a Hyprnote pull request ([fastrepl/anarlog#1417](https://github.com/fastrepl/anarlog/pull/1417), closed unmerged) built a token trie with whisper-rs, but never boosted the first token of a term, used the greedy tokenizer and had no word boundary check [docs]. [CTranslate2 PR #1789](https://github.com/OpenNMT/CTranslate2/pull/1789) adds sequence biasing for Whisper and has been open since 2024 [docs]. The zero shot trie paper by Liu, Peng and Chng is summarized under [Research evidence on vocabulary biasing](#research-evidence-on-vocabulary-biasing).

#### Voice activity detection

- whisper.cpp ships Silero VAD support. The current ggml model is `ggml-silero-v6.2.0.bin`, 885,098 bytes, from [ggml-org/whisper-vad](https://huggingface.co/ggml-org/whisper-vad) [code]. Defaults: threshold 0.5, minimum speech 250 ms, minimum silence 100 ms, padding 30 ms [code].
- The built in path runs only inside `whisper_full` and `whisper_full_parallel` on the default state. It runs on the CPU, joins the speech segments into one buffer with 0.1 s of silence between them, and decodes that buffer in 30 s windows. It removes silence but does not split decoding at pauses [code].
- The standalone `whisper_vad_*` API gives segments and per window probabilities, and `whisper_vad_detect_speech_no_reset` keeps state for streaming end of speech detection [code].
- Hallucination on silence is structural: `suppress_blank` forces text as the first token, and the no speech filter needs two conditions at once [code].
- In Biorouter's benchmark, whisper.cpp without VAD produced text on silence or noise in almost every run ("you", "Thank you for watching!"), and with a glossary prompt it emitted dictionary terms such as `PHD2, PHD2, PHD2`. With Silero VAD on, all 24 decodes of silence and noise were empty, and recall on speech stayed within 2 points [measured, synthetic speech].

#### The examples: stream, command and server

- **`stream`** transcribes the last 10 s again every 3 s with 200 ms of overlap, or waits for an energy VAD to see silence and then transcribes the utterance. It captures through SDL2, and its README calls it naive [code].
- **`command`** classifies speech against a list of allowed commands: one decode step, then a softmax over the first tokens of each command [code].
- **`server`** offers `POST /inference`, `POST /load` and `GET /health` [code]. It has **no authentication** and sends `Access-Control-Allow-Origin: *` [code]. A multipart POST needs no CORS preflight, so any web page can send audio to a loopback whisper-server and read the transcript [inferred from CORS rules]. `/load` takes any server side file path, frees the model, and calls `exit(1)` if the new file fails to load [code]. One global mutex serializes requests [code]. It decodes WAV, FLAC, MP3 and Vorbis; WebM and Opus need `--convert`, which shells out to `ffmpeg` [code]. Request fields include `prompt` and `carry_initial_prompt` but no logit bias field [code]. A Unix socket is possible on macOS and Linux only [code].

#### Models, quantizations and memory

The canonical ggml weights are on Hugging Face at [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1), frozen since 2024-10-29 at revision `5359861c`. Every file has a SHA-256 (the LFS object id) and a SHA-1 published in the README, and both were checked on downloaded files [measured]. Selected files [docs, Hugging Face API]:

| File | Bytes | Languages | Fits one 2 GiB GitHub release asset |
|---|---:|---|---|
| `ggml-tiny.en.bin` | 77,704,715 | English | yes |
| `ggml-base.en.bin` | 147,964,211 | English | yes |
| `ggml-base.en-q8_0.bin` | 81,781,811 | English | yes |
| `ggml-small.en.bin` | 487,614,201 | English | yes |
| `ggml-small.en-q8_0.bin` | 264,477,561 | English | yes |
| `ggml-small.en-q5_1.bin` | 190,098,681 | English | yes |
| `ggml-medium.en-q5_0.bin` | 539,225,533 | English | yes |
| `ggml-large-v3-turbo.bin` | 1,624,555,275 | 99 languages | yes |
| `ggml-large-v3-turbo-q8_0.bin` | 874,188,075 | 99 languages | yes |
| `ggml-large-v3-turbo-q5_0.bin` | 574,041,195 | 99 languages | yes |
| `ggml-large-v3.bin` | 3,095,033,483 | 99 languages | no |
| `ggml-large-v3-q5_0.bin` | 1,081,140,203 | 99 languages | yes |

- Memory from the whisper.cpp README: tiny about 273 MB, base 388 MB, small 852 MB, medium 2.1 GB, large 3.9 GB [docs]. Beam search multiplies the self attention cache by the number of decoders plus two [code].
- `large-v3-turbo` keeps the 32 layer large encoder and has 4 decoder layers instead of 32. Encoding costs the same as `large-v3`, and decoding is 5 to 7 times cheaper [docs].
- The `.en` models are clearly better at tiny and base sizes, and the advantage fades at small and medium [docs].
- No published comparison of q5_0 or q8_0 WER against f16 was found. On Metal, q5_0 made the encoder slightly slower (M2 Ultra large-v2: 249 to 286 ms), so quantization saves disk and memory, not GPU time [docs].
- Reported encoder time for one 30 s window on an Apple M3 Pro with Metal: tiny 34 ms, base 59 ms, small 200 ms, medium 535 ms, large-v3 969 ms [docs, issue #89]. On a Ryzen 9 5950X CPU: base 427 ms, small 1,471 ms, medium 4,418 ms [docs].
- Core ML encoder packages (`*-encoder.mlmodelc.zip`) exist for every size. The turbo one is 1.17 GB, about twice the q5_0 model it speeds up. The first run compiles for the Neural Engine and is slow. macOS 14 or later is recommended. The turbo package came in as a community contribution [docs]. A newer ANEForge encoder path produces bundles tied to one machine and OS build, so they cannot be prebuilt [docs].
- Distil-Whisper publishes English only f16 ggml conversions (`distil-large-v3.5`, 1.52 GB) and no quantized ones [docs].

#### Backends

| Backend | Platforms | Notes |
|---|---|---|
| Metal | Apple Silicon | Shader library can be embedded, so no `.metallib` ships |
| Accelerate BLAS | macOS | Imports symbols that need macOS 13.3; OpenWhispr turns BLAS off for its macOS 12 arm64 build and raises its x64 floor to 13.3 instead |
| Core ML | macOS | Encoder only; a missing package fails state creation unless fallback is compiled in |
| Vulkan | Windows, Linux | Needs the Vulkan SDK to build; quantized model garbage was reported once (#3047); the first start compiles shaders and can take minutes |
| CUDA, ROCm, OpenBLAS, OpenVINO | various | CUDA builds are 285 to 685 MB |
| CPU | all | `GGML_NATIVE` defaults to on, which compiles for the build machine's CPU and can crash older CPUs with SIGILL. Runtime CPU dispatch (`GGML_CPU_ALL_VARIANTS`) needs `GGML_BACKEND_DL` and shared libraries |

A missing GPU falls back to the CPU silently; only a log line says so [code]. On Intel Macs, ggml's Metal shader compile never finishes on some AMD GPUs, which is why Handy ships Intel Macs CPU only [docs, Handy issue #2340].

#### Packaging and distribution

- The v1.9.5 release carries **no** binaries. The matching nightly `b5454` has an xcframework, Linux CPU tarballs and Windows CPU, BLAS and CUDA zips. There is no macOS command line or server binary and no Vulkan build [docs, GitHub API].
- The Linux tarball needs `GLIBC_2.34` and `GLIBCXX_3.4.30` [measured, symbol table read]. That excludes Debian 11, and through the C++ library version probably Rocky 9 too [inferred from distribution versions]. The Windows zip lacks the Visual C++ runtime DLLs and a license file; on a machine without the redistributable the loader stops the program with `0xC0000135` before `main` [docs, OpenWhispr's code comments].
- About 2,200 `GGML_ASSERT` and `GGML_ABORT` sites in core ggml plus the CPU backend end in `abort()`, about 2,450 with the Metal backend added, and about 5,500 across all backends in the tree, because each GPU backend adds its own; whisper.cpp adds 23 `WHISPER_ASSERT` sites [code, counted]. `ggml_abort()` ends in `abort()` even when an abort callback is set [code]. ggml installs a `std::terminate` handler from a static initializer, and on Linux an abort forks `gdb` to print a backtrace unless `GGML_NO_BACKTRACE` is set [code]. A crash inside the host process takes the host down.
- The project does not accept pull requests that are fully or mostly AI generated (`AGENTS.md`) [code], so fixes a downstream project needs are more likely to live in its own patch series.

**What to take.**

- Pin a commit or tag, and build the engine yourself for each platform with `GGML_NATIVE=OFF`.
- Capture PCM, split the recording into chunks of 30 s or less at VAD pauses, and decode each chunk with its own call so the prompt applies to every chunk.
- Gate decoding on VAD and return nothing when there is no speech.
- Count prompt tokens with the real tokenizer, stay near 200, and put the most important terms **last**.
- Keep temperature low, and remember that biasing silently stops under fallback.
- Treat `logits_filter_callback` as the route to a dictionary larger than the prompt, matched on bytes, with the boost limited to tokens the audio already makes plausible.

**What to avoid.**

- Shipping `whisper-server` unpatched on a loopback port.
- Shipping upstream prebuilt binaries as they are.
- Linking ggml into a long running process that also does other work.
- `suppress_nst` and grammars for open dictation.
- Assuming the prompt was applied.

### whisper-rs

**What it is.** Rust bindings for whisper.cpp. The project moved to [Codeberg](https://codeberg.org/tazz4843/whisper-rs); the GitHub repository is a frozen stub at 0.14.3 (2025-07-30) [code]. The latest crates are `whisper-rs` 0.16.0 and `whisper-rs-sys` 0.15.0 (2026-03-12). The last Codeberg commit was 2026-03-14 [docs]. **License:** Unlicense.

**Engine and models.** `whisper-rs-sys` 0.15.0 vendors **whisper.cpp 1.8.3**, so it lacks the 1.9.x fixes (VAD on empty input, the streaming VAD API, decoder seeding) unless the sys crate is pointed at another whisper.cpp [code]. The safe API exposes `set_initial_prompt`, `set_tokens`, the unsafe `set_filter_logits_callback`, `set_grammar` and the VAD setters. It has no `set_carry_initial_prompt` and no `suppress_regex` [docs, docs.rs].

**Build.** CMake plus bindgen, which needs `libclang`. The fallback bindings shipped in the crate were generated on Linux with glibc 2.42, where `c_long` is 64 bit; that type is wrong for Windows [code]. The build script does not set `GGML_NATIVE`, and it tests `cfg!(target_os)` inside the build script, which describes the build host, not the target [code]. No public report of a Linux to MinGW cross build was found [docs]. Whispering disabled whisper.cpp on Windows because `whisper-rs-sys` conflicted with other native crates over the MSVC C runtime (static against dynamic) [code, Whispering `Cargo.toml`].

**Known issues.** Issue #277: the safe abort callback has a trampoline type mismatch that reads past its heap buffer; its fix, PR #278, has waited since 2026-04-29 [docs]. The maintainer's README states opposition to generative AI tools, which matters for a project that would upstream fixes written with them [code].

**What to take.** The API surface is the right one, including the logits callback. Building it inside a separate helper, natively per platform, avoids most of the cross compile problems.

**What to avoid.** Depending on the vendored 1.8.3. Assuming bindgen output is portable. Trusting the safe abort callback until #277 is fixed.

### faster-whisper

**What it is.** A Whisper reimplementation on CTranslate2 by SYSTRAN, in Python. It reports being up to 4 times faster than openai/whisper at the same accuracy, with 8 bit quantization on CPU and GPU [docs]. Version 1.2.1. **License:** MIT.

**Engine and models.** CTranslate2 conversions, downloaded at runtime from the Hugging Face organization `Systran` (`Systran/faster-whisper-small.en` and so on) [code]. Audio is decoded with PyAV, which bundles FFmpeg libraries [docs]. GPU use needs CUDA 12 and cuDNN 9 [docs].

**Vocabulary.** Three prompt slots behave differently [code]:

| Slot | Placement | Truncation | Lifetime |
|---|---|---|---|
| `hotwords` | After `<\|startofprev\|>`, before history, in every 30 s window | Keeps the **head** (first 223 tokens) | Survives temperature fallback; skipped only in the first window when `prefix` is set |
| `initial_prompt` | Seeds the rolling history | History keeps the **tail** | Dropped when `condition_on_previous_text` is false or fallback passes 0.5 |
| `prefix` | After the start sequence | 223 tokens | Forces the transcript to begin with this text, first window only; not a biasing tool |

- `hotwords` came from PR [#731](https://github.com/SYSTRAN/faster-whisper/pull/731) (merged 2024-05-04), because `initial_prompt` applied only to the first window [docs].
- The prompt plus `max_new_tokens` must stay within 448 positions, or the call raises `ValueError` (issues [#1022](https://github.com/SYSTRAN/faster-whisper/issues/1022) and [#948](https://github.com/SYSTRAN/faster-whisper/issues/948)) [code].
- The batched pipeline encodes `initial_prompt` without the leading space the sequential path adds [code].
- Issue [#1356](https://github.com/SYSTRAN/faster-whisper/issues/1356) reports silent audio producing the hotwords themselves. The maintainer's answer was to enable `vad_filter=True` [docs]. A Biorouter smoke test reproduced it: 3 s of faint noise with an 11 term `hotwords` list produced a long loop of dictionary words that took 12 s to decode; with a 1,011 term list, truncation kept the first 223 tokens and every real term was dropped [measured, small.en int8 on an Apple M4 Max CPU, synthetic speech].

**Capture and VAD.** The library takes files or arrays; capture is the caller's job. Silero VAD is built in through `vad_filter=True`, and by default it removes only silence longer than 2 s [docs].

**Speed.** On CPU, small.en int8 took at least about 0.9 s per call, even for a 2 s clip, because the encoder always processes a padded 30 s window. A 20 s clip took about 2 s [measured, same smoke test].

**What to take.** The static per window `hotwords` slot shows how to keep a glossary applied across windows. The 448 position budget is a hard limit to check before decoding.

**What to avoid.** Head truncation of a long list. Runtime model downloads from a third party. A Python dependency inside a Rust and Electron product.

### whisper_streaming

**What it is.** A streaming policy for Whisper from ÚFAL (Charles University), with a paper reporting 3.3 s latency on long unsegmented speech [docs, [ACL Anthology 2023.ijcnlp-demo.3](https://aclanthology.org/2023.ijcnlp-demo.3/)]. The README says it is being superseded by [SimulStreaming](https://github.com/ufal/SimulStreaming) [code]. **License:** MIT.

**Engine.** faster-whisper by default (large-v2, beam 5, word timestamps), with other backends possible [code].

**How it streams** [code]:

- Every iteration, after at least 1 s of new audio, it transcribes the **whole** buffer again.
- **LocalAgreement-2** commits the longest common prefix of two consecutive hypotheses, compared word by word. A word appears only once two iterations agree.
- The buffer is trimmed at a completed segment once it passes 15 s, or at a sentence boundary.
- The prompt is up to 200 characters of committed text that has scrolled out of the buffer.
- An optional Silero mode finalizes an utterance after 500 ms of silence.

**Vocabulary.** None built in. The rolling prompt is the only lever [code].

**What to take.** LocalAgreement is a clean commit rule if a live preview is ever wanted.

**What to avoid.** Using it for push to talk dictation: one second of speech can be encoded about 15 times, and committed text lags by at least two iterations [inferred from the loop].

### WhisperLive

**What it is.** A WebSocket transcription server with clients, from Collabora. Version 0.10.0. **License:** MIT.

**Engine and models.** faster-whisper by default (`small.en`; CUDA or CPU int8), plus OpenVINO and TensorRT backends. Models download from Hugging Face at runtime, with automatic CTranslate2 conversion for nonstandard ids [code].

**Protocol** [code]:

- The first text frame is JSON options, including `language`, `model`, `use_vad`, `initial_prompt`, `hotwords`, `no_speech_thresh` and `vad_parameters`.
- Audio follows as binary frames (float32 by default), and `END_OF_AUDIO` ends the stream.
- The server answers with segments that carry a `completed` flag, and it resends the last 10 segments each time, so a client must **replace** text rather than append it.
- An incomplete segment that repeats 7 to 10 times unchanged is committed by force. The buffer is capped at 45 s by dropping the oldest 30 s.

**Vocabulary.** `initial_prompt` and `hotwords` pass straight to faster-whisper on every call. Committed text is not fed back as a prompt [code].

**What to take.** The protocol shape: an options frame, a completion flag, and replace semantics for revised text.

**What to avoid.** Runtime model fetching from a third party. Partial text that must be revised in place inside an editable field.

### WhisperKit

**What it is.** Argmax's Swift framework for Whisper on Core ML, now part of the Argmax open source SDK (the repository is presented as `argmax-oss-swift`). Its README lists macOS 14 or later and Xcode 16 as prerequisites [docs], but the package declares a macOS 13 minimum and falls back on systems older than 14 [code]. **License:** MIT.

**Engine and models.** Core ML Whisper models from Hugging Face `argmaxinc/whisperkit-coreml`. The endpoint is configurable (`modelEndpoint`, `downloadBase`, `modelRepo`), which is a precedent for a self hosted mirror [code]. The `argmax-cli` tool can stream from the microphone, and it includes a local server that implements the OpenAI audio API with streamed output [docs].

**Vocabulary** [code]:

- `promptTokens` are prepended after `<|startofprev|>` and `prefixTokens` are appended after the start sequence. `suppressTokens` and pluggable `logitsFilters` exist.
- The prompt budget is **111 tokens**, half of whisper.cpp's, and the suffix is kept.
- The prompt is static for every 30 s window, so it behaves like whisper.cpp with `carry_initial_prompt` on.
- An empty prompt skips `<|startofprev|>` entirely, because "a bare `<|startofprev|>` with no content tokens biases the model toward ending the segment early".
- The command line tool and server encode `" " + prompt`, so the first word tokenizes as a word initial token.
- Real custom vocabulary is a closed Pro SDK feature. Argmax describes it as a keyword search over the audio whose hits are merged into any model's transcript, supporting 1,000 keywords. Argmax reports keyword F1 rising from 64% to 92% on a name subset of earnings22 [docs, [Argmax blog](https://www.argmaxinc.com/blog/customize-speech-to-text)].

**What to take.** A static prompt per window. The leading space. Skipping the prompt marker when the prompt is empty. A configurable model endpoint. A keyword pass that works after decoding, whatever the model.

**What to avoid.** Depending on closed features. It is Apple only and Swift, so it is design reference for a Rust and Electron product, not reusable code.

### sherpa-onnx

**What it is.** The k2-fsa project's speech toolkit on ONNX Runtime: recognition, VAD (Silero and TEN), keyword spotting, command line tools, WebSocket servers, and bindings for many languages. It has an official Rust crate, `sherpa-onnx` 2.0.1 (2026-10-27). The older third party `sherpa-rs` is deprecated and its repository was archived on 2026-06-06 [docs]. **License:** Apache-2.0.

**Engine and models.** Zipformer and Paraformer transducers, NeMo TDT, RNNT and CTC models (Parakeet, Nemotron), Canary, Whisper in ONNX form, Moonshine, SenseVoice, Qwen3-ASR, Fun-ASR and MedASR [code]. Models are distributed as **GitHub release assets**: the `asr-models` tag holds at least 497 assets totalling 187 GB, the largest just under 2 GiB, and `silero_vad.onnx` (643,854 bytes) has 1.12 million downloads [docs, GitHub API].

**Capture and streaming.** The project ships microphone examples. The official Rust example for Parakeet runs Silero VAD and decodes the whole buffered utterance again every 0.2 s for a live display, then decodes the finished segment once more; a 5 s cap on speech segments bounds that repeated work [code]. Cache aware streaming transducers such as Nemotron decode chunk by chunk instead [code].

**Vocabulary: hotwords as a context graph** [code]:

- Hotwords form an Aho-Corasick trie over token ids. During `modified_beam_search`, a token that extends a phrase gets a bonus. If the match then fails, the partial bonus is taken back. A completed phrase keeps its bonus.
- For NeMo TDT offline decoding there is also a selection boost, so a low probability continuation of a hotword can enter the beam.
- Hotwords can be set per stream with `create_stream_with_hotwords`, phrases separated by `/`, and they are merged with a global file. Each line takes an optional `:score`. The default score is 1.5; the Rust crate's default of 0.0 silently means 1.5.
- English BPE models need `modeling_unit="bpe"` and a `bpe_vocab` file, because the default is `"cjkchar"`. The released Parakeet packages do not include `bpe.vocab`; it must be generated once from the NeMo checkpoint.
- Matching is case sensitive on tokens, so a hotword must be written in the form the model emits.

Hotword support by model family [code]:

| Family | Hotwords |
|---|---|
| Zipformer, conformer and LSTM transducers | yes |
| NeMo transducers: Parakeet TDT v2 and v3, Nemotron streaming | yes; offline since PR #3077 (2026-02-05), streaming since PR #3895 (2026-09-14) |
| Qwen3-ASR, Fun-ASR-Nano | prompt based: terms go into the system turn; a warning past 48 tokens; PR #3907 fixed hallucination on silence when hotwords are set |
| Whisper, Moonshine, SenseVoice, every CTC model, Parakeet Unified streaming | no (greedy only) |

**Packaging.** `sherpa-onnx-sys` downloads prebuilt native libraries from GitHub releases **at build time**, with no checksum check, unless `SHERPA_ONNX_LIB_DIR` points at a local copy [code]. Static library archives for 2.0.1 are 20 to 23 MB for macOS and Linux and 123 MB for Windows (MSVC) [docs]. About 1,000 `SHERPA_ONNX_EXIT` sites call `_Exit` (997 in `sherpa-onnx/csrc`, about 700 of them outside the NPU backends), which ends the host process on a bad model path or configuration [code].

**Measured behavior.** In a Biorouter smoke test on an Apple M4 Max CPU with synthetic speech and Parakeet TDT 0.6B v2 int8 [measured]:

- Hotwords at score 1.5 fixed EPAS1, SPOKE, OMOP and Baranzini.
- With 1,011 hotwords, every real term was still corrected, the per stream graph took 2 to 7 ms to build, and decoding took about 25 ms per second of audio.
- At score 3.0, capitalization drifted ("Screen Used Guide") and faint noise produced a false accept ("Uh easy."). The 1,011 term list produced a phantom sentence on noise.
- The streaming Nemotron model wrote numbers as words ("TP fifty three", "HIF two A"), so a written form hotword `TP53` cannot match it. Adding the spoken alias "EPAS one" made it emit "EPAS one", which a replacement pass must then map to EPAS1.

**What to take.**

- A decoding time biasing mechanism that scales to thousands of terms with per term weights.
- Per stream lists, so the dictionary can change without a restart.
- Storing the form the model emits, not only the display form.

**What to avoid.**

- Unpinned build time downloads.
- Running the library inside a process that must not exit.
- High scores and very long lists.
- The `cjkchar` default for English.

### transcribe.cpp

**What it is.** A ggml based C and C++ STT library from handy-computer, the organization behind Handy. The repository was created 2026-04-07 and published 21 releases (24 version tags, v0.0.1 to v0.4.1) in about 6.5 months, the latest 0.4.1 on 2026-10-22 [docs]. **License:** MIT, with ggml, Silero VAD and miniz under MIT.

**Engine and models.** More than 16 families and 60 variants in GGUF: Whisper (including turbo), Parakeet (CTC, RNNT, TDT, Unified), Nemotron streaming, Canary, Canary-Qwen, Cohere Transcribe, Qwen3-ASR, Granite Speech, Voxtral, Moonshine, SenseVoice, Fun-ASR, GigaAM, MOSS and Google's MedASR. VAD, language identification and diarization models also run as roles. The project says each published GGUF is checked numerically and tested for WER against its reference implementation. It also loads legacy whisper.cpp `.bin` files [docs]. Models are hosted in the Hugging Face organization `handy-computer`.

**Packaging.** The Rust crate `transcribe-cpp` commits its bindgen output, so it needs no `libclang`, and `TRANSCRIBE_DIR` links a prebuilt install instead of building from source [code]. Each release ships native bundles with digests: macOS arm64 Metal 2.5 MB, macOS x86_64 CPU 1.6 MB, Windows x86_64 CPU plus Vulkan 18.1 MB, Linux x86_64 CPU plus Vulkan 22.6 MB, and CUDA bundles near 203 MB [docs]. Windows builds use MSVC, and the build script has no MinGW path [code]. There is no HTTP server tool [code]. The API is still before 1.0: versions 0.2 and 0.4 each shipped a migration guide, and a binding must match the contract version exactly [code].

**Capture and streaming.** The library does not capture audio; the caller passes 16 kHz mono float PCM. Streaming models expose a session that takes chunks and returns committed and tentative text, which Handy shows in its overlay [code].

**Vocabulary.** Generic run parameters, with a feature bit per model [code, [prompting doc](https://github.com/handy-computer/transcribe.cpp/blob/5700f127e38dd53c4735ea23ef743b4b538eacd4/docs/prompting.md)]:

| Family | `vocabulary` | Context `prompt` | `prefix` |
|---|---|---|---|
| Whisper | yes | yes | yes |
| Qwen3-ASR | yes | yes | no |
| Granite `4.0-1b` and `4.1-2b` | yes | no | no (yes on `4.1-2b-plus`) |
| Fun-ASR-Nano, MOSS | yes | no | no |
| Canary | no | no | yes |
| Parakeet, Nemotron, Cohere, Moonshine, SenseVoice, MedASR | no | no | no |

- `vocabulary` is a list of terms in priority order, formatted per family. For Whisper it goes into the `<|startofprev|>` slot.
- Because Whisper keeps the last tokens, which would drop the highest priority terms, transcribe.cpp trims the context text first, then drops terms from the **end** of the priority list, with one warning [code].
- Control token literals in terms are rejected [code].
- For Qwen3-ASR, terms are joined with spaces rather than commas. A code comment says the authors measured this as better, with fewer cases of the whole dictionary dumped into the output [code].
- The Qwen3-ASR 0.6B model card warns that this size "echoes the dictionary into the output more often than the 1.7B" [code].

**Speed data.** The catalog records speed for every model through one runtime, measured on 2026-09-14 on an Apple M4, an M4 Max and a Ryzen 7 PRO 4750U laptop; the table under [Speech models beyond Whisper](#speech-models-beyond-whisper) uses it [docs].

**What to take.** Capability bits per family instead of assuming every model takes a prompt. A priority ordered vocabulary with an explicit overflow policy. One runtime for several model families.

**What to avoid.** Assuming Parakeet gets decoder biasing here: it does not. Treating a young API from one small vendor as stable.

### Other runtimes

- **llama.cpp** at build b11429 (2026-10-05) runs audio models through `libmtmd`: Qwen3-ASR (PR #19441), Granite Speech (PR #22101), Voxtral Mini 2507, Gemma 4 audio, LFM2-Audio, Ultravox and GLM-ASR [code].
  - `llama-server` exposes an OpenAI compatible `POST /v1/audio/transcriptions` (PR #21863, 2026-04-14). Its `prompt` field replaces the default user instruction, "Transcribe audio to text", and only `json` responses are accepted [code].
  - `/v1/chat/completions` accepts `input_audio` content parts, which is the route to put a dictionary into Qwen3-ASR's system message [inferred].
  - The documentation still calls audio "highly experimental" [code].
  - Streaming audio input and a `/v1/realtime` endpoint landed after b11429 (2026-10-15 and 2026-10-20) [docs].
  - It cannot run Whisper, standalone Parakeet, Canary or Moonshine [code].
- **NeMo-Speech.cpp** (NVIDIA, v0.3.0, Apache-2.0) has a C interface and an OpenAI compatible HTTP server for Parakeet TDT v3, Parakeet CTC 1.1B and Nemotron streaming. Its `speech_contexts` boosting works for cache aware RNNT and CTC with a language model, and its documentation says Parakeet TDT ignores it [code].
- **NVIDIA NeMo GPU-PB** (phrase boosting) works for CTC, RNNT, TDT and AED models and scales to about 20,000 phrases, but it is Python and CUDA only [docs, [NeMo word boosting](https://docs.nvidia.com/nemo/speech/latest/asr/asr_customization/word_boosting.html)].

## Dictation applications

### OpenAI Codex

**What it is.** OpenAI's coding agent. The open repository holds the CLI and TUI (Rust); the desktop app and IDE extension are closed source [code]. **License:** Apache-2.0 for the open repository. Codex has three different voice features, and only one of them is dictation [code, docs].

#### Terminal realtime voice

`/voice` (or F8) starts a full duplex speech to speech conversation with a realtime model over WebRTC. It is not dictation: speech becomes a spoken user turn, not composer text [code]. The engineering is still instructive:

- **A separate helper process**, `codex-voice-host`, holds all native audio: cpal capture, a rubato resampler to 48 kHz, a pure Rust port of WebRTC audio processing (echo cancellation, noise suppression, gain), Opus encoding in 20 ms frames, and WebRTC transport [code].
- **Least privilege.** The helper is signed with only the macOS `com.apple.security.device.audio-input` entitlement, and audio never crosses the control pipe; the parent sees only level meters [code].
- **Control protocol** [code]:
  - Length prefixed JSON frames over stdio, at most 128 KiB.
  - A same build handshake: the helper refuses a parent with a different build commit.
  - An allowlisted child environment, and a helper path resolved only inside the install.
  - stderr discarded so native error text cannot leak.
  - Parent death detected through stdin end of file, with a hard exit after 2 s.
  - Exit codes 20 to 39 name the failing stage, and the parent maps them to fixed sentences such as "voice audio devices could not open; check microphone and speaker setup".
- **Mute generations.** Each device has a counter where odd means disabled. On unmute, audio captured before the cutoff is rejected, and while muted the helper sends generated silence, never device audio [code]. The README admits this "can discard speech onset" [code].
- **Platform details.** Linux callbacks are 50 ms because a 20 ms ALSA ring can be shorter than one PipeWire cycle. There is no hot plug support; losing the device ends the session [code].
- **Screen lock** stops voice and prints "Voice stopped because your session locked." [code].

#### The removed composer dictation

PR [#3381](https://github.com/openai/codex/pull/3381) (merged 2026-02-23) added hold Space to dictate into the TUI composer. PR [#16114](https://github.com/openai/codex/pull/16114) removed it on 2026-03-29 as unfinished [docs]. It is the closest open source prior art to a composer microphone [code, at commit `855e2755`]:

- cpal default device at its native rate, 16 bit WAV peak normalized to 90% of full scale, and clips under 1.0 s discarded.
- Upload to OpenAI's `/v1/audio/transcriptions` with `gpt-4o-transcribe`, with **`prompt` set to the composer text before the cursor**. That is OpenAI's own use of context biasing.
- An anchor element inserted at the caret showed a level meter, then a spinner, and was replaced in place by the transcript wherever it had moved.
- Its weaknesses: every failure was silent (the error field was dead code), a short clip vanished without a word, there was no cancel gesture, overloading Space needed timing heuristics, and Linux was compiled out.

#### Desktop app dictation

Closed source, described here from about 35 user issues and OpenAI's documentation [docs]:

- A **Dictate** button beside Send. A click toggles recording; a press longer than 150 ms is push to talk. The default shortcut is Ctrl+M.
- While recording, a waveform strip, a square stop control beside Send, and a cancel X. The transcript lands as editable composer text.
- An administrator gate, `in_app_dictation`, can switch it off [code, feature flag in the open repository].
- The issue tracker is a catalogue of failures, almost all of them lost speech or unclear state:

| Failure | Issues |
|---|---|
| Listening shown before capture was ready; the first 0.5 to 3 s lost (median 587 ms, measured by a user from saved audio) | #57400, #58448, #58499 |
| A 179 ms click crossed the 150 ms hold threshold and the recording was discarded without a message | #58685 |
| Enter triggered cancel and discarded a long dictation | #59463 |
| Switching tab, thread or window stopped recording and lost the audio | #52856, #29951, #54282 |
| The auto updater restarted the app mid recording | #48431 |
| One shortcut started dictation in two composers | #23398, #37623 |
| Text inserted at the end instead of the caret | #57401, #59413 |
| A failed realtime start left the microphone held ("Microphone is already in use") | #55745 |
| A flat meter was the only sign of an OS level mute | #59108, #43018 |
| Word by word streaming lowered accuracy for long technical dictation; users asked to bring back "record first, process the whole utterance, insert a refined final transcript" | #56075 |
| Phantom "Yeah" text with no speech | #36701 |
| Domain terms misheard, with a request to use installed skill names as recognition context | #45186, #53957 |

#### OpenAI's transcription APIs

From the [speech to text guide](https://developers.openai.com/api/docs/guides/speech-to-text) and the [realtime transcription guide](https://developers.openai.com/api/docs/guides/realtime-transcription) [docs]:

- `gpt-transcribe` takes a free text `prompt`, a `keywords` list ("literal terms you expect to hear", "hints, not required output") and `languages`. A keyword containing `<`, `>`, CR or LF rejects the whole request.
- `whisper-1`, `gpt-4o-transcribe` and `gpt-4o-mini-transcribe` are deprecated and shut down on 2027-02-26.
- The `whisper-1` prompt is limited to 224 tokens and suits a small list of correct spellings. For longer lists, OpenAI suggests correcting the text with a text model afterward.
- Realtime transcription uses `gpt-live-transcribe`, PCM at 24 kHz, client side VAD, and the same `prompt`, `keywords` and `languages` fields.

#### What the TUI does well

Patterns pinned by snapshot tests in the open repository [code]:

- A distinct "connecting" state with no meter; the red dot appears only when the microphone is truly live.
- Level meters sampled on a fixed 100 ms clock, not on redraw.
- The stop control keeps its position and is the last thing to disappear at narrow widths.
- The terminal title gains a recording dot while the microphone is live.
- Settings say "Applies to your next voice conversation." and persist on the machine.
- The `/voice` command is hidden unless the voice package is physically present on disk.
- Every refused start says why.

**What to take.**

- Context biasing with the text already typed, and a separate keyword list for the dictionary.
- A supervised helper with a strict control protocol and fixed error sentences.
- A generation counter so audio from before a press is never transcribed.
- An arming state until the first real audio arrives.
- Insertion at the caret through an anchor that tracks edits.
- Availability as a function of what is installed.

**What to avoid.**

- Silent discard of any audio.
- A hold threshold short enough to catch clicks.
- Enter mapped to cancel.
- Recording that dies on navigation or updates.
- Two composers answering one shortcut.
- Word by word partials written into the editable text as the only mode.

### Handy

**What it is.** A desktop dictation app for macOS, Windows and Linux on Tauri 2 (Rust backend, React frontend) that types into any application. Version 0.10.0 (2026-11-02). **License:** MIT.

**Engine and models** [code]:

- Since 0.9.0 every model runs through transcribe.cpp; 0.10.0 removed ONNX support.
- **All native engine code runs in a child process**: the Handy executable relaunched with a hidden `--transcribe-worker` argument.
  - Frames are `[u32 json length][json][u32 pcm length][f32 PCM]` over stdio.
  - Deadlines scale with audio length: GPU floor 30 s or 60 s, CPU floor 120 s or 300 s.
  - A GPU fault or hang is retried in a CPU only worker.
  - Unloading a model kills the worker, which returns all memory to the OS.
  - Debug builds inject faults (`HANDY_WORKER_FAULT=abort@load` and similar).
- Backends: static Metal on Apple Silicon, CPU only on Intel Macs (Metal shader compile hang, #2340), Vulkan with per instruction set CPU modules on Windows x64 and Linux, CPU on Windows arm64.
- The model unloads after an idle timeout (default 5 minutes) and preloads when recording starts.

**Capture and VAD** [code]:

- Capture uses `handy-recorder`, extracted from Handy: 16 kHz mono at the VAD frame size, rubato FFT resampling, and reports of dropped audio, device loss and an all zero (muted) input instead of silent failure.
- VAD is Silero v6.2 as a 1.24 MB GGUF **bundled** with the app: threshold 0.3, 450 ms preroll, 60 ms onset, 450 ms hangover (1,650 ms for streaming models). Earshot, a pure Rust VAD, is the fallback.
- Audio of 40 s or less is decoded in one piece; longer audio is cut at VAD pauses into chunks of about 30 s.
- The overlay stays grey ("arming") until the first real audio chunk arrives, and the start chime plays then. This fixed lost first words on Windows (#1879).

**Modes and insertion.** Toggle, push to talk, and `HoldOrToggle` (the default, decided by a 300 ms hold threshold). Global shortcuts need macOS Accessibility permission. Text goes in through the clipboard and a synthetic paste, with clipboard restore [code]. An app that inserts into its own text field needs none of this.

**Model catalog and hosting** [code]:

- A catalog compiled into the binary from the npm package `@handy-computer/models` 0.2.3. Each model lists repository, **pinned revision**, license, capabilities, and per quantization file name, size and SHA-256, plus FLEURS English WER and speed on a Ryzen 4750U laptop.
- Primary source: Hugging Face `handy-computer`, at the pinned revision.
- Fallback mirror: `https://blob.handy.computer` on Cloudflare R2, with object keys of repository, revision and file name, and immutable cache headers. A mirror script hashes every file against the catalog before upload and can stream every object back to recheck it.
- The SHA-256 compiled into the binary is the trust anchor, and the mirror is treated as an untrusted pipe.
- Gap: the primary Hugging Face path does not hash the file in Handy's own code.

**Download paths** [code]. The two sources run different code with different rules.

The primary Hugging Face path (`fetch_hf_file`) downloads through a fork of `hf-hub` into the shared Hugging Face cache:

- Up to four attempts with a stream schedule of 4, then 1, 1 and 1, because some networks reset parallel connections (#1579).
- A watchdog cancels an attempt that makes no progress for 60 s. Resume comes from `hf-hub`'s own `.sync.part` marker, not from a `.partial` file.
- It sets no connect timeout of its own and checks no hash.
- Ambient Hugging Face tokens are ignored, because a stale cached token broke public downloads.

The direct HTTP path (`download_http_resumable`) serves the mirror fallback and models whose source is a URL. It is resumable, into `.partial`, then a rename:

- A partial already at full size is verified rather than requested again.
- A `200` reply to a range request restarts from zero.
- A `206` reply must start exactly at the requested offset.
- A server advertising a different total than the catalog is rejected.
- A 15 s connect timeout applies, and every body chunk has a 60 s stall deadline.
- SHA-256 is checked on a worker thread, a mismatch deletes the file, and cancel keeps the partial.

**Vocabulary** [code]:

- `custom_words` is a flat list of strings with no aliases. An Advanced setting adds removable chips; a hidden Debug panel holds the 0.18 correction threshold, which the maintainer says is "finicky to set properly".
- When the loaded model reports `Feature::Vocabulary` (Whisper, Qwen3-ASR, Granite), the words go to the engine and fuzzy correction is **skipped**. Otherwise, for example on Parakeet, the default model, a fuzzy pass runs:
  - sequences of up to 3 words, never across punctuation;
  - normalized Levenshtein distance, multiplied by 0.3 when Soundex codes match;
  - a 25% length guard and ASCII only;
  - the original token's case pattern is kept.
- Soundex is never applied to terms with digits, so gene symbols get edit distance only. The streaming preview does not apply custom words, and an optional LLM cleanup step does not receive the list.
- A Python port run on biomedical terms [measured] fixed "EPAS 1" and "T P 53". It could not map spoken digits ("e pass one") or letters said as words ("you see SF"). At a higher threshold it ate neighbouring words ("I work at UCSF" became "I work UCSF"), and Soundex truncation produced a false positive ("crisper screen" became "CRISPR scRNA-seq").
- Community proposals: learn from the person's corrections and per token confidence (#2310), import lists made from documents (#2379), and short lived context terms (#2189).

**What to take.**

- The worker process pattern with deadlines scaled to audio length and a CPU retry.
- A bundled VAD model with preroll and hangover.
- The arming cue.
- A compiled in catalog with pinned revisions, sizes and hashes.
- A self hosted mirror treated as untrusted.
- The direct HTTP path's resumable download rules, with its hash check applied to every source.
- Gating features on the model's capability bits.

**What to avoid.**

- An either or rule between engine biasing and text correction: the prompt can be dropped, so correction should always run.
- A hidden accuracy knob.
- A Soundex only matcher for alphanumeric terms.
- Skipping the hash on any source.

### VoiceInk

**What it is.** A native macOS dictation app in Swift for macOS 15 and later, sold with a license key and a free trial. The README says it does not accept pull requests. Version 2.25 (2026-11-03 release "Improved Dictionary Auto Learning") [code, docs]. **License:** GPL-3.0, so its code, including close ports of its algorithms, cannot enter a project under another license; only its ideas can [inferred from the license].

**Engine and models** [code]:

- Local whisper.cpp, Parakeet and Nemotron through FluidAudio on Core ML, transcribe.cpp GGUF models (Cohere Transcribe, SenseVoice), Apple's `SpeechAnalyzer` on macOS 26, and about a dozen cloud providers.
- Whisper models download from Hugging Face `ggerganov/whisper.cpp` at `main`, with **no hash or size check** and no resume. The transcribe.cpp models and VoiceInk's own LLM cleanup model ("VoiceInk Refine", 1.06 GB, 16 GiB memory needed) pin a revision, an exact size and a SHA-256.
- Silero VAD v5.1.2 (885,098 bytes) is bundled and on by default: threshold 0.5, minimum speech 250 ms, minimum silence 100 ms.

**Pipeline** [code]: transcribe, filter, vocabulary correction, formatting, word replacement, optional LLM enhancement, delivery, save. The output filter removes tags such as `[BLANK_AUDIO]`, anything in brackets, parentheses or braces, and filler words. The parentheses rule also deletes dictated parentheticals.

**Vocabulary: three mechanisms** [code, docs, [Vocabulary docs](https://tryvoiceink.com/docs/vocabulary)]:

1. **Vocabulary**: preferred spellings, stored as a term and a date.
   - For whisper.cpp the prompt is a short per language style sentence, a newline, then the terms joined with commas.
   - Terms are sorted alphabetically, and terms OCR'd from the screen are appended after them. Because whisper.cpp keeps the tail, an overlong list loses the style sentence and the alphabetically earliest saved terms first, while the screen names survive [inferred from both codes].
   - Cloud providers get the list in their own hint format with their own caps (Groq 896 bytes, Deepgram 100 terms). Local Parakeet, transcribe.cpp and Apple engines get no vocabulary.
2. **Phonetic matcher** after recognition, for every engine.
   - Terms must be lowercase a to z words. Any token with digits, symbols, inner periods or characters outside ASCII is never rewritten, so EPAS1, IL-6 and scRNA-seq are excluded.
   - The cost mixes spelling distance and a Metaphone style sound distance.
   - It **abstains** when the runner up is within 0.04 of the best, refuses single words of 3 letters or fewer, and protects terms already spelled correctly.
   - An exact match in the wrong case (`ucsf`) is not recased.
3. **Word Replacements**: deterministic substitutions from many comma separated sources to one destination.
   - Case insensitive match with word boundaries; the destination is inserted verbatim.
   - Longest source first, applied in sequence, which is why writes run cycle detection.
   - A source that already maps to a different destination is refused, and rules with the same destination merge.

**Screen names.** With Screen Recording permission, VoiceInk OCRs the front window and adds names found by Apple's name tagger to that one recording's vocabulary. This is a short lived, context derived list merged with the saved one [code].

**Auto Learn** (on by default) [code, [Auto Learn docs](https://tryvoiceink.com/docs/auto-learn-dictionary)]:

- After pasting, VoiceInk reads the edited field through macOS Accessibility for up to 60 s and diffs it into small hunks: at most 3 words and 64 characters each, no commas, no control characters.
- A local reviewer rejects function words, inflections and contraction expansions. An optional AI reviewer classifies each pair as `reusable`, `specific` or `uncertain` under a strict JSON contract, and treats all input as data.
- At most 3 suggestions per observation. A toast asks **Add Correction** or **Dismiss** and disappears after 10 s; a dismissed pair never returns.
- Auto Learn only ever proposes Word Replacements, never Vocabulary.

**Import, export and editor** [code]:

- JSON with a stable format id (`voiceink.dictionary`) and a schema version independent of the app version.
- Import offers merge and replace, with a dry run preview that counts duplicates, conflicts, invalid, comma containing and cyclic entries before writing.
- The editor is a sidebar item, **Dictionary**, with a **Word Replacements | Vocabulary** switch, comma separated bulk add, a sort toggle, and removable chips. Vocabulary chips cannot be edited in place.

**Privacy.** The whole vocabulary goes to cloud STT and LLM providers when they are selected [code]. For Biorouter, whose privacy tiers are a safety boundary against mistakes, that would be a tier crossing.

**What to take.**

- Separate recognition hints from deterministic replacements.
- Compare sources case folded and destinations case sensitively.
- Abstain on ambiguity in fuzzy matching.
- Learn only from narrow edits, as suggestions, with dismissals remembered.
- A versioned export format and a dry run import.
- A per recording context list.

**What to avoid.**

- Alphabetical prompt order, and no token counting.
- Unhashed Whisper downloads.
- Deleting all parenthesized text.
- Sequential replacement that feeds rules into each other.
- Copying any code.

### OpenWhispr

**What it is.** An Electron dictation app (React 19, Electron 43) with a global hotkey, an in chat composer and local or cloud engines. Version 1.11.0 (2026-10-29). **License:** MIT. Of the apps surveyed, it is the closest stack to Biorouter's desktop app.

**Engine and binaries** [code]:

- whisper.cpp runs as a `whisper-server` sidecar built in OpenWhispr's own fork, published as release assets of `OpenWhispr/whisper.cpp` tag [0.0.11](https://github.com/OpenWhispr/whisper.cpp/releases/tag/0.0.11) (2026-10-21).
- CPU and Metal builds are fetched at **build time** and bundled. The macOS arm64 server is one 3.65 MB Mach-O with whisper and ggml linked statically; it links only system frameworks.
- CUDA and Vulkan packs download at runtime with pinned SHA-256 and fail closed on mismatch ("The binaries are unsigned and release assets are mutable").
- The bundled CPU binaries are trusted by tag alone.
- The pinned tag exists because "tracking the latest release let an upstream whisper.cpp bump change transcription output between app releases with no diff to review".
- The fork is 31 commits ahead and 1,861 behind upstream, which shows how a long lived fork drifts [docs, GitHub compare].
- Build lessons from its workflow:
  - `-DGGML_NATIVE=OFF` in every job except the macOS arm64 (Metal) job, which builds the arm64 server above.
  - `GGML_BLAS=OFF` only in the macOS arm64 build, which targets macOS 12.0, because `ggml-blas` imports `$NEWLAPACK$ILP64` symbols that need macOS 13.3. The macOS x64 build keeps BLAS and raises its floor to 13.3. The arm64 binary still links `Accelerate.framework`.
  - `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` is a silent no op unless CMake policy CMP0091 is set.
  - Without the bundled MSVC runtime DLLs, Windows users without the redistributable hit `0xC0000135`.
  - The Linux CPU build on Ubuntu 22.04 still needs glibc 2.34.

**Server settings** [code]:

- Bound to 127.0.0.1 on a port from 8178 to 8199. A 30 s startup timeout, 120 s for Vulkan shader compilation. Prewarmed at app start.
- `--max-len 4096` instead of `--no-timestamps`. whisper.cpp 1.9.x forces token timestamps with a maximum length of 60, which breaks words, and `--no-timestamps` advances a full 30 s window, which silently skips audio on long takes (OpenWhispr #2150).
- `entropy_thold=2.8` and `logprob_thold=-1.25`. A code comment reports that these settings cut hallucinated endings from 2.25% to 0.06% of 4,814 real dictations [docs, code comment].
- A timeout that scales with audio length. No cancellation during a decode. A crashed GPU server is retried on CPU.
- On Windows, whisper-server reads arguments in the ANSI code page and crashes on a VAD model path outside ASCII, so models live under an ASCII safe folder.
- VAD is **off by default for dictation**: on pause heavy speech it stripped real audio and left Whisper decoding near silence seeded with the dictionary, which produced dictionary words (#1454). Biorouter's benchmark on synthetic speech saw no recall loss with VAD on, so real speech with long pauses is the case still to test [measured, inferred].

**Capture** [code]:

- `getUserMedia` with echo cancellation, noise suppression and automatic gain all off. On Windows, Chromium's AGC changes the system microphone volume (#476). Stereo is requested because mono WebM broke silence detection under PipeWire (#472).
- A 16 kHz PCM tap through an AudioWorklet loaded from a `blob:` URL, which a strict `script-src` refuses, with `MediaRecorder` WebM plus FFmpeg as the fallback.
- A speech gate skips decoding when the take is silent: peak RMS below 0.002.
- Microphone errors are mapped by `DOMException.name`: `NotAllowedError` means permission denied, `NotReadableError` means the device is busy.
- On macOS, the main process calls `systemPreferences.askForMediaAccess("microphone")` before `getUserMedia`.

**Model downloads.** From Hugging Face `ggerganov/whisper.cpp` at `main`, resumable, with retries, a stall timeout, a disk space check at 1.2 times the model size, one download at a time, and an atomic rename. Verification is **size only**: at least 90% of the expected size [code].

**Dictionary** [code]:

- SQLite table of words with a `source` of `manual` or `learned`, soft delete, a 5,000 word cap, and an identity key of NFC normalized, trimmed, lowercased text.
- Writes are deltas (`{add, remove}`), never whole lists, because "a caller holding a stale snapshot deletes everything it omitted" (#1295).
- **Prompt builder.** A code comment says Whisper decoders "read at most 223 prompt tokens and keep the TAIL", and that a comma separated list of names takes about 2.3 to 2.9 characters per token, so the window is worth about 550 characters. Tiers fill greedily by priority and are emitted lowest tier first, so the person's own words sit at the tail. The UI warns past the limit.
- **Echo guard.** Three checks run against the prompt that was sent: a whole prompt echo (at least 90% of unique words are dictionary words and at least 70% of dictionary words appear), a fragment echo that continues the prompt's shape, and a trailing recital after real speech.
- **Echo recovery.** On detection it retries once with no prompt and no VAD, and adopts the retry only if it no longer looks like the prompt. A discarded echo is reported as "No audio detected".
- **Learning from edits**: word level LCS alignment; it rejects rewrites (more than 50% of words changed), words under 3 letters, common words and pairs with normalized edit distance above 0.65. A toast with **Undo** follows.
- **Agent tool** `update_dictionary` with `add[]` and `remove[]`. Removals apply first, so a respelling is one call. Results report `added`, `removed`, `alreadyPresent` and `notFound`. The system prompt tells the agent to propose removals and wait for confirmation.
- Dictionary words given to an LLM are wrapped as "spelling hints ... not instructions", and `<`, `>` and newlines are stripped.

**In chat composer** [code]:

- The microphone takes the Send button's circle when the input is empty, and Send replaces it when there is text.
- Recording shows a waveform, a timer, Cancel and Stop; then "Transcribing".
- The transcript is **appended** at the end, not at the caret.
- The voice draft deliberately skips the LLM cleanup pass.

**Streaming.** In July 2026 OpenWhispr shipped Nemotron streaming through sherpa-onnx, committing the streamed text at stop. It still recommends offline Parakeet "when accuracy matters most": 6.93% WER for streaming against 5.91% for the best batch model [docs, [OpenWhispr blog](https://openwhispr.com/blog/local-streaming-speech-to-text)].

**What to take.**

- A pinned engine build per platform.
- Hallucination thresholds measured on real dictations.
- Capture with browser processing off.
- The speech gate.
- The prompt budget in characters and tail placement.
- The echo guard and its retry.
- Delta writes for the dictionary.
- The agent tool contract.
- Wrapping terms as data for any LLM.

**What to avoid.**

- Size only checks.
- Runtime downloads from third parties.
- A `blob:` worklet under a strict CSP.
- Appending at the end instead of the caret.
- Unhashed bundled binaries.

### Whispering (Epicenter)

**What it is.** A Tauri 2 dictation app from Epicenter. Version 7.11.0 (2025-12-27) was the last with STT engines running inside the app. HEAD (7.11.1, 2026-10-17) is a rewrite into a minimal voice notes app that only calls one OpenAI compatible endpoint the person chooses. That endpoint may be a hosted service or a local server, and a keyless local endpoint is allowed. The API key never reaches the page and requests use `redirect: 'error'` and `credentials: 'omit'` [code]. **License:** AGPL-3.0, so ideas only.

**Engine in v7.11.0** [code]:

- In process Rust through `transcribe-rs` 0.2.1: whisper-rs plus ONNX Parakeet and Moonshine.
- A 5 minute idle unload. A poisoned mutex clears the engine and forces a reload.
- On Windows only Parakeet was available, because `whisper-rs-sys` and other native crates conflicted over static and dynamic MSVC runtimes.

**Capture** [code]:

- Three recorder backends: cpal in Rust (the default, writing WAV), the WebView's `MediaRecorder`, or the `ffmpeg` command.
- A VAD mode with `@ricky0123/vad-web` that turns each detected utterance into its own WAV.
- Compressed recordings need a system FFmpeg, and the error message says so.
- The renderer sent audio to Rust as a JSON array of numbers, a slow encoding.

**Vocabulary.** No dictionary, only a free text "System Prompt" passed as `initial_prompt`, or as Deepgram `keyterm`. A capability table marks `supportsPrompt: false` for Parakeet and Moonshine. Ordered "transformations" add `find_replace` steps (plain or regex) and LLM `prompt_transform` steps [code].

**Model hosting.** Parakeet TDT 0.6B v3 int8 is hosted as release [`models/parakeet-tdt-0.6b-v3-int8`](https://github.com/EpicenterHQ/epicenter/releases/tag/models/parakeet-tdt-0.6b-v3-int8) in Epicenter's own GitHub repository: encoder 652 MB, decoder 18 MB and a 479 MB tarball [docs, GitHub API]. Whisper models still come from Hugging Face. Downloads happen in the renderer, with no resume and a size only check of at least 90% [code].

**What to take.**

- Hosting models as GitHub release assets in the project's own organization.
- Deterministic find and replace steps as a cheap complement to prompting.
- A capability flag per engine.

**What to avoid.**

- Linking whisper.cpp into the main process on Windows.
- Number array IPC.
- Size only checks.
- Depending on a system FFmpeg.

### whisper-writer

**What it is.** A small Python app that records after a keyboard shortcut (Ctrl+Shift+Space by default) and types the transcript into the active window. The last commit was 2024-08-24 [code]. **License:** GPL-3.0.

**Engine.** faster-whisper locally, or OpenAI's API, with `condition_on_previous_text` on and an optional VAD filter [code].

**Capture and modes** [code]:

- `sounddevice` int16 mono at 16 kHz.
- WebRTC VAD at aggressiveness 2 with 30 ms frames, stopping after 900 ms of silence.
- Recordings under 100 ms are discarded, and the first 150 ms is skipped to drop the key click.
- Four modes: `continuous`, `voice_activity_detection`, `press_to_toggle` and `hold_to_record`.

**Vocabulary.** A free text `initial_prompt` only [code].

**What to take.** Skipping the first 150 ms to drop the key click. A plain list of recording modes.

**What to avoid.** It is unmaintained since 2024, and the license allows ideas only.

### Buzz

**What it is.** A PyQt desktop app for transcribing files and live microphone audio offline, with a command line interface. **License:** MIT.

**Engine and models** [code]:

- Several backends: whisper.cpp (with Vulkan), faster-whisper, Hugging Face transformers models and the OpenAI API.
- It bundles whisper.cpp's `whisper-server` and starts it on a random free port with `--inference-path /audio/transcriptions`, which is OpenAI compatible, plus `--no-timestamps`, `--max-context 8`, `--entropy-thold 2.8` and `--suppress-nst`.
- Models come from Hugging Face `ggerganov/whisper.cpp` through `huggingface_hub`.
- The Core ML encoder name drops the quantization suffix, matching whisper.cpp's pairing rule.

**Live mode** [code]:

- 5 s batches, cut at the last quiet 20 ms window, with a backlog capped at 3 batches.
- On the in process whisper and faster-whisper backends, each next chunk's prompt **replaces** the user's initial prompt with the previous chunk's text, so a vocabulary given as the initial prompt conditions only the first chunk. The `whisper-server` path ignores that rolling text and sends the user's initial prompt with every chunk.

**What to take.** An OpenAI compatible inference path. A random port.

**What to avoid.**

- Replacing the vocabulary prompt with rolling text.
- `--no-timestamps` on long input.
- Exposing an unauthenticated server.

## Speech models beyond Whisper

Whisper is not the most accurate open model any more, and several newer models take a vocabulary in other ways. The figures below come from the [Open ASR Leaderboard](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard) as of its "02-11-2026" version, which averages nine English datasets and dropped LibriSpeech on 2026-11-02, so older vendor numbers are lower and not comparable [docs]. Its speed column is GPU batch throughput and says nothing about a laptop. FLEURS English WER comes from Handy's catalog at Q8_0 [docs]. Read speech is the closest proxy for dictation, so the leaderboard's private scripted column is shown too.

| Model | Params | Board average WER | Private scripted | FLEURS en | License | Native vocabulary input |
|---|---|---:|---:|---:|---|---|
| Qwen3-ASR-1.7B | 2.04B | 6.14 | 2.87 | 3.23 | Apache-2.0 | Free text context in the system message |
| Granite Speech 5.0 2B | about 2.3B | 6.59 | 3.16 | n/a | Apache-2.0 | `Keywords:` list, up to 2,000 terms |
| Cohere Transcribe 03-2026 | 2.05B | 6.72 | 3.20 | 5.08 | Apache-2.0 (gated download) | none |
| Canary-Qwen-2.5B | 2.53B | 6.83 | 3.47 | 3.58 | CC-BY-4.0 | none trained |
| Parakeet TDT 0.6B v2 | 0.62B | 6.91 | 3.62 | 4.11 | CC-BY-4.0 | none; hotwords only through sherpa-onnx or NeMo |
| Granite Speech 4.1 2B | 2.31B | 6.92 | 3.16 | 4.14 | Apache-2.0 | `Keywords:` list |
| Qwen3-ASR-0.6B | 0.78B | 6.99 | 3.92 | 4.23 | Apache-2.0 | Free text context |
| Parakeet TDT 0.6B v3 | 0.63B | 7.03 | 4.27 | 4.83 | CC-BY-4.0 | as v2 |
| Nemotron speech streaming EN 0.6B | 0.62B | 7.37 | 4.79 | 6.43 | NVIDIA Open Model License | NeMo-Speech.cpp and sherpa-onnx boosting |
| distil-large-v3.5 | 0.76B | 7.55 | 4.34 | n/a | MIT | Whisper prompt |
| Whisper large-v3 | 1.54B | 8.18 | 3.61 | 4.03 | MIT | Prompt, 223 tokens |
| Voxtral Mini 3B 2507 | 4.68B with encoder | 8.29 | 3.73 | 3.89 | Apache-2.0 | none ("System prompts are not yet supported") |
| Canary 1B v2 | 0.98B | 8.35 | 4.65 | 4.47 | CC-BY-4.0 | prefix only |
| Whisper large-v3-turbo | 0.81B | 8.68 | 4.05 | 4.38 | MIT | Prompt, 223 tokens |
| Voxtral Mini 4B Realtime 2602 | 4.43B | 9.59 | 5.76 | 6.27 | Apache-2.0 | none in open weights |
| Nemotron 3.5 ASR streaming 0.6B | 0.64B | 10.85 | 7.84 | 7.90 | OpenMDW-1.1 | NeMo-Speech.cpp boosting |

The closed APIs at the top of the full board lead by about 1.3 points, and every serious open candidate at 2B parameters or less sits within about one point of the others (6.1 to 7.4). The larger gaps are on proper nouns and jargon, which is the dictionary's job [docs, inferred]. A vendor run medical term benchmark from Omi Health, with its conflict of interest disclosed, put Qwen3-ASR-1.7B first among small open models at about 4.40% medical WER and Parakeet v2 at about 6.19% [docs, secondhand].

### Speed on laptops

All rows come from one runtime (transcribe.cpp, Q8_0, a 35.3 s clip, measured 2026-09-14) [docs, transcribe.cpp catalog]:

| Model | Apple M4 CPU | Apple M4 Metal | Ryzen 7 4750U CPU | Q8_0 size |
|---|---:|---:|---:|---:|
| Parakeet TDT CTC 110M | 95.5× | 194.7× | 43.9× | 135 MB |
| Parakeet TDT 0.6B v2 | 26.9× | 82.4× | 12.6× | 730 MB |
| Parakeet Unified EN 0.6B | 24.6× | 74.6× | 12.3× | 731 MB |
| Nemotron streaming EN 0.6B | 25.7× | 76.1× | 12.2× | 730 MB |
| Canary 180M Flash | 46.0× | 63.8× | 17.8× | 218 MB |
| Moonshine streaming medium | 18.0× | 13.3× | 5.9× | 296 MB |
| SenseVoice Small | 32.9× | 114.7× | 18.6× | 253 MB |
| Qwen3-ASR-0.6B | 12.0× | 23.0× | 4.6× | 850 MB |
| Cohere Transcribe | 8.3× | 28.3× | 4.0× | 2.41 GB |
| Whisper large-v3-turbo | 4.0× | 15.0× | 1.7× | 886 MB |
| Qwen3-ASR-1.7B | 6.1× | 10.5× | 2.25× | 2.19 GB |
| Granite Speech 4.1 2B | 4.5× | 9.0× | 1.7× | 2.56 GB |
| Voxtral Mini 3B 2507 | 1.7× | 4.2× | 0.63× | 5.0 GB |

Apart from Voxtral, every model here turns a 15 s push to talk utterance into text within about 2 s on Apple Silicon with Metal [inferred from the table]. On a 2020 laptop CPU, Qwen3-ASR-1.7B and Whisper turbo become slow: a 15 s utterance takes about 6.7 s and 9 s [inferred from the table]. Load time is separate and is avoided by keeping the model resident.

### Model profiles

- **NVIDIA Parakeet** (FastConformer encoder).
  - TDT 0.6B v2 is English only with built in punctuation and casing. v3 covers 25 European languages with automatic language identification. Parakeet Unified EN 0.6B is one model for offline and streaming, under the NVIDIA Open Model License; Handy's catalog labels it the best general choice for English dictation [docs]. A 110M TDT CTC variant is small and fast.
  - No prompt input. Biasing comes only from decoder boosting in sherpa-onnx (TDT through `modified_beam_search`) or NeMo GPU-PB. whisper.cpp's Parakeet API, transcribe.cpp and NeMo-Speech.cpp give TDT no biasing [code].
  - The `ggml-org/parakeet-GGUF` card says MIT, but a conversion cannot relicense CC-BY-4.0 weights, so attribution and a note of changes are still owed [inferred].
- **Qwen3-ASR** (Alibaba Qwen, 2026-01-28, Apache-2.0).
  - 0.6B and 1.7B, 30 languages and 22 Chinese dialects, strong punctuation and casing.
  - Context is free text in the system message, before an audio only user turn [code, [Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR/tree/7c6daf77a2421100f5fb066495372c00129d39ff)]. Output takes the form `language English<asr_text>...`.
  - It runs in llama.cpp. The GGUF pair for 0.6B is 805 MB plus a 214 MB projector. The 1.7B Q8_0 is 2,165 MB, over GitHub's 2 GiB asset limit, so Q6_K (about 1.7 GB) or a split file is needed [docs].
  - Known failure: echoing the dictionary into the output, worse at 0.6B (transcribe.cpp note; sherpa-onnx #3907; OpenWhispr #2581 on 1.7B) [code, docs].
- **IBM Granite Speech** (Apache-2.0).
  - A speech LLM with a trained keyword input: `"<|audio|>transcribe the speech to text. Keywords: <kw1>, <kw2>, ..."`. Version 5.0 accepts up to 2,000 keywords [docs, model cards].
  - Secondary coverage cites Earnings-22 keyword F1 rising from 67.2 to 90.1 with correct lists [docs, secondhand].
  - Runs in llama.cpp (1.96 GB Q8_0 plus a 1.16 GB f16 projector) and transcribe.cpp. It is roughly 2.5 times slower than Qwen3-ASR-0.6B at similar English WER [docs, transcribe.cpp catalog].
  - The TurboCTC 470M variants are fast and take no keywords, and the `-nc` variant is CC-BY-NC-SA.
- **NVIDIA Canary.** Canary 1B v2 covers 25 European languages plus translation, and Canary 180M Flash covers English, German, Spanish and French at 218 MB; both are CC-BY-4.0. Canary-Qwen-2.5B pairs the encoder with Qwen3-1.7B and was trained on audio up to 40 s. None takes a trained context input; transcribe.cpp supports only a transcript prefix, and NeMo GPU-PB supports AED models. The original `canary-1b` is noncommercial [docs].
- **Moonshine** (Moonshine AI).
  - The v2 streaming models are Tiny 34M, Small 123M and Medium 245M, English, with cased and punctuated output. Code and models are MIT, except legacy non streaming models for other languages [code].
  - **Key terms** (`keyterms`) or a **context** passage feed a prefix tree over subwords with a logit bonus of `boost * (1 + ln(depth))`. The default boost is 2.0, and lists can change while audio streams. Context mode extracts single rare words and skips words with digits, so terms like IPv6 must be listed outright. Only the streaming architectures support it [code].
  - The vendor's measurements on LibriSpeech with 100 term lists [docs, [domain customization](https://github.com/moonshine-ai/moonshine/blob/234f60faa0eb388b01cdf7e60aca232af37aefda/docs/models/domain-customization.md)]:
    - Medium term WER fell from 7.89% to 5.58% at the default boost, while other words moved from 3.90% to 3.93%.
    - On Tiny, 1,000 unused terms raised WER from 4.83% to 5.88% at the default boost, and 10,000 to 6.09%.
    - On Tiny, a boost of 4 with 100 unused terms cost about five and a half points, and at a boost of 6 decoding fell apart on every model.
  - The library is C++ on ONNX Runtime with Python, Swift, Android and JavaScript bindings and a C API, and no Rust binding. It ships microphone samples for each platform, including a dictation demo for the web, and `moonshine-voice mic` transcribes from the command line [code].
  - Models download on first use from `download.moonshine.ai`, and `modelsFrom(path)` points the engine at local files for offline builds [code].
- **Nemotron speech streaming** (NVIDIA). A cache aware RNNT with fixed chunk sizes (80 to 1,120 ms for the English model). In Biorouter's smoke test it wrote numbers as words, dropped the first word without 0.5 s of preroll, and its partial text flickered ("EPAS", then "EPA's one") as hotword bonuses resolved [measured]. The English model is under the NVIDIA Open Model License, which requires a notice file and terminates on guardrail bypass [docs].
- **Mistral Voxtral.** Mini 3B 2507 takes no system prompt and is slow (4.2× on M4 Metal). Mini 4B Realtime streams with a configurable 240 ms to 2.4 s delay, but is weaker in English and needs a llama.cpp build newer than b11429 [docs].
- **Cohere Transcribe** (2B, 14 languages, Apache-2.0, gated download). Prompting only toggles punctuation; there is no context biasing [docs].
- **Distil-Whisper.** distil-large-v3.5 (MIT, English) biases like Whisper, has the full large encoder, so short dictation latency is close to turbo's, and ships only as f16 ggml [docs, inferred].
- **Kyutai STT.** 1B (English and French, 0.5 s delay) and 2.6B (English, 2.5 s delay), CC-BY-4.0, with no biasing [docs]. The 2.5 s delay suits captions better than dictation [inferred].
- **SenseVoice and Fun-ASR-Nano** (FunAudioLLM). SenseVoice Small is very fast but uses a nonstandard model license with an anti denigration clause and no biasing. Fun-ASR-Nano takes hotwords; its license is Apache-2.0 on Hugging Face and "other" in transcribe.cpp's catalog, a conflict to resolve [docs].
- **Google MedASR.** A 105M CTC model trained on about 5,000 hours of physician dictation (radiology, internal medicine, family medicine). Google's model card reports 6.6% to 9.3% WER with greedy decoding on its internal dictation sets, and 4.6% to 6.9% when an external word language model (order 6) is added during decoding. The card gives no FLEURS figure; Handy's catalog measures 37.5% WER on general FLEURS English for the Q8_0 GGUF. It is gated under the Health AI Developer Foundations terms, whose use restrictions must flow down to users, so it cannot be rehosted freely. As a CTC model it gets no hotwords in sherpa-onnx [docs].
- **Others.**
  - Microsoft VibeVoice-ASR (8.7B, MIT, hotwords) is too large.
  - Cactus Whistle (16.9 MB, Apache-2.0, `keywords=[...]`) has vendor numbers only.
  - Gemma 4 audio and Phi-4-multimodal are chat models with weak ASR.
  - Liquid LFM2-Audio has a revenue capped license, and FUTO and ABR models use custom licenses [docs].

### Vocabulary support by model and runtime

| Model family | Native mechanism | whisper.cpp | llama.cpp b11429 | transcribe.cpp 0.4 | sherpa-onnx 2.0.1 | NeMo-Speech.cpp 0.3 | Own library |
|---|---|---|---|---|---|---|---|
| Whisper, Distil-Whisper | Prompt, 223 tokens | `prompt` | not supported | `vocabulary`, `prompt`, `prefix` | no hotwords | not supported | faster-whisper `hotwords` |
| Parakeet TDT v2, v3 | none | no | not supported | no | hotwords | ignored for TDT | NeMo GPU-PB (Python) |
| Nemotron streaming | none | not supported | not supported | no | hotwords (streaming) | `speech_contexts` | NeMo |
| Canary | prefix only | not supported | not supported | `prefix` | no | not supported | NeMo GPU-PB |
| Moonshine v2 streaming | key terms, context | not supported | not supported | no | no | not supported | Moonshine Voice C API |
| Qwen3-ASR | system message context | not supported | yes, through the system message [inferred] | `vocabulary`, `prompt` | hotwords (system prompt) | not supported | qwen-asr (Python) |
| Granite Speech | `Keywords:` list | not supported | yes, as prompt text [inferred] | `vocabulary` | not supported | not supported | transformers |
| Voxtral, Cohere | none | not supported | instruct only (Voxtral) | instruct only | no | not supported | closed API only |

### Licenses for rehosting

| License | Models | Conditions |
|---|---|---|
| MIT | Whisper (all sizes), Distil-Whisper, Moonshine, ggml conversions, Silero VAD | Ship the notice with every copy, including the copy installed on a person's machine |
| Apache-2.0 | Qwen3-ASR, Granite (except `-nc`), Voxtral, Cohere | Keep LICENSE and NOTICE; state changes such as quantization |
| CC-BY-4.0 | Parakeet, Canary, Kyutai | Attribution, a license link and an indication of changes; a GGUF conversion is an adaptation |
| NVIDIA Open Model License | Parakeet Unified, Nemotron streaming EN | A copy of the agreement and a notice file; rights end on guardrail bypass or litigation |
| OpenMDW-1.1 | Nemotron 3.5 | Similar to MIT; keep the notices |
| Avoid for a default | CC-BY-NC variants, MedASR (HAI-DEF), SenseVoice (FunASR model license), LFM1.0, FUTO and ABR | Noncommercial, flow down or nonstandard terms |

GitHub release assets must each be under 2 GiB, with no total size or bandwidth limit [docs, [About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)]. That excludes f16 `large-v3` (2.9 GB) and Q8_0 Qwen3-ASR-1.7B as single files. Git LFS on a free organization plan allows 10 GiB of bandwidth a month [docs], which is about 18 downloads of a 547 MiB model [inferred].

## Vocabulary in commercial products and services

These products are closed, so only their documented behavior is known [docs]. They matter because their limits and warnings are the best industry evidence of where vocabulary lists stop helping.

| Product | Recognition hints | Replacement after recognition | Learning | Limits and warnings |
|---|---|---|---|---|
| [Wispr Flow](https://docs.wisprflow.ai/articles/4052411709-teach-flow-your-words-with-the-dictionary) | Dictionary words | Corrections: whole word, case insensitive, saved capitalization kept, longer phrases first | Learns only from edits to dictated text; skips paths, web addresses, IDs, numbers, everyday words and very common names; a deleted word is never added again | 60 characters per field; a team dictionary that personal entries override; administrators can turn it off |
| [Superwhisper](https://superwhisper.com/docs/get-started/interface-vocabulary) | Vocabulary sent as hints | Replacements "applied programmatically, with no AI interpretation", in exact case | none built in | "Keep the vocabulary list short"; too many custom words can cause hallucinations |
| MacWhisper | A prompt setting | Global Replace with a whole word option | none | |
| Dragon Medical One | Written form plus spoken form ("Pronunciation"), optional audio training | Auto texts | Desktop Dragon learns from documents and sent mail | Site and group vocabularies beside personal ones; import format `written\\spoken` |
| [Talon community](https://github.com/talonhub/community/tree/ab4b5069da7d684168f91f68f543c30d2501024b) (MIT) | `vocabulary.talon-list` maps spoken to written | `words_to_replace.csv`, longest phrase first | Voice commands copy a selection into either list | The clearest open split between a recognition list and a replacement list [code] |
| [AWS Transcribe](https://docs.aws.amazon.com/transcribe/latest/dg/custom-vocabulary-create-table.html) | Custom vocabulary: `Phrase` is the spoken form, with acronym letters separated by periods and digits spelled out (`VX02Q` is `V.X.-zero-two-Q.`); `DisplayAs` is the written form | n/a | n/a | `SoundsLike` and `IPA` columns are no longer supported; do not enter PHI in a vocabulary |
| Apple Speech | `contextualStrings`; custom language models with X-SAMPA pronunciations | n/a | Custom models can be generated at runtime | macOS only, on the device only |
| [Deepgram](https://developers.deepgram.com/docs/keyterm) | `keyterm`: up to 100 terms and 500 tokens; recommends 20 to 50 | n/a | n/a | Write proper nouns capitalized; a `term:2` weight is treated as literal text |
| AssemblyAI | `keyterms_prompt`: 1,000 terms of up to 6 words (asynchronous) | `custom_spelling` | n/a | |
| Google Cloud Speech, Azure Speech | Phrase boosts: Google up to a practical maximum of 20; Azure weight 0 to 2, at most 200 phrases recommended | n/a | n/a | Both warn that higher boosts raise false positives |

## Research evidence on vocabulary biasing

| Work | Setting | Result |
|---|---|---|
| Jogi et al., [arXiv 2502.11572](https://arxiv.org/abs/2502.11572) | Whisper large, a 70 word list in the prompt, 11 datasets | Rare word WER fell from 23.7 to 18.0, but WER on unbiased words rose from 9.1 to 11.6, and overall WER rose on 6 of 11 datasets |
| Hou et al., Interspeech 2025 | Whisper turbo, lists with up to 2,000 distractors | A raw 100 word list cut biased WER by about 25%; 500 or more words helped negligibly; the top 50 chosen by a scorer cut it by about 40%; **true words at the end of the list** worked best |
| Liu, Peng and Chng, [arXiv 2508.17796](https://arxiv.org/abs/2508.17796) | Frozen Whisper large-v3, beam 10, a prefix trie with pronunciation variants made by TTS | Biased WER down 43% and 44% with unbiased WER unchanged, no training; unfiltered variants pushed unbiased WER to 21.19% |
| TurboBias (NVIDIA), [arXiv 2508.07014](https://arxiv.org/abs/2508.07014) | GPU phrase boosting for CTC, RNNT and AED | On a medical set with 991 phrases, RNNT beam key phrase F rose from 62.5 to 75.8; precision dropped about 5 points; boosting can outscore end of sentence in AED and cause hallucination |
| Pusateri et al. (Apple), [arXiv 2409.06062](https://arxiv.org/abs/2409.06062) | Retrieval augmented correction of named entities | Phonetic retrieval beat text retrieval (top 1 recall 84.8% against 53.5%); one candidate beat five because extra candidates confused the corrector |
| CB-Whisper, [arXiv 2309.09552](https://arxiv.org/abs/2309.09552) | Keyword spotting, then prompting only the detected entities with spoken form hints | Better entity recall than plain concatenated lists, which hallucinate more |
| Careless Whisper, [arXiv 2402.08021](https://arxiv.org/abs/2402.08021) | Whisper API (2023) on aphasic and control speech | 1.4% of segments contained hallucinated text, and 38% of those hallucinations contained explicit harms |

Biorouter's own analysis of the HGNC gene list adds three facts that explain why off the shelf fuzzy matchers fail on biomedical terms [measured, HGNC complete set downloaded 2026-11-04]:

- 86.2% of the 19,299 protein coding gene symbols contain a digit, and 60% have the exact shape of letters then digits (EPAS1). VoiceInk never rewrites such tokens, and Handy gives them no phonetic boost.
- At least 149 protein coding symbols are ordinary English words, such as SET, CAT, MAX, REST and KIT.
- 427 aliases equal another gene's approved symbol.

## Comparison

| Project | Kind | License | Engine and models | Model source and integrity | Capture and UX | Vocabulary | Main lesson |
|---|---|---|---|---|---|---|---|
| whisper.cpp | Engine | MIT | Whisper ggml `.bin`, Parakeet; Metal, Vulkan, CUDA, CPU | Hugging Face, SHA-256 and SHA-1 published; no macOS or Vulkan release binaries | `stream` and `command` examples; server without auth | Prompt (223 tokens, tail kept), logits callback, grammar | Build and pin it yourself; gate on VAD; keep ggml out of the host process |
| whisper-rs | Rust bindings | Unlicense | whisper.cpp 1.8.3 vendored | n/a | n/a | Prompt, tokens, unsafe logits callback | Useful API, but it lags upstream and has an open memory bug |
| faster-whisper | Python engine | MIT | CTranslate2 Whisper | Hugging Face `Systran`, runtime download | Library only | `hotwords` (head kept), `initial_prompt` (tail kept), `prefix` | Two slots with opposite truncation; hotwords hallucinate on silence |
| whisper_streaming | Streaming policy | MIT | faster-whisper | Hugging Face | Live captions | Rolling prompt only | LocalAgreement commit rule; too costly for push to talk |
| WhisperLive | Server | MIT | faster-whisper, OpenVINO, TensorRT | Hugging Face, runtime conversion | WebSocket with `completed` flags | `initial_prompt`, `hotwords` per call | Clients must replace revised segments |
| WhisperKit | Swift framework | MIT | Whisper on Core ML | Hugging Face, endpoint configurable | CLI and local OpenAI style server | `promptTokens` (111 tokens), closed keyword feature | Static prompt per window; leading space |
| sherpa-onnx | Engine | Apache-2.0 | ONNX transducers, CTC, Whisper, Qwen3-ASR, Moonshine | GitHub release assets as a model CDN | CLI, WebSocket servers, mobile demos | Aho-Corasick hotwords for transducers, per stream | Decoding time biasing that scales; pin build time downloads |
| transcribe.cpp | Engine | MIT | GGUF, 16+ families | Hugging Face `handy-computer` with hashes; prebuilt native bundles | Library only | `vocabulary` with priority order and explicit overflow | Capability bits per family |
| Moonshine Voice | Library and models | MIT | Own streaming models on ONNX Runtime | `download.moonshine.ai`, or local files | Microphone samples for many platforms | Key terms and context, trie bonus | Published tradeoff tables for boost and list length |
| OpenAI Codex | Agent | Apache-2.0 (open part) | Cloud only (realtime and transcription APIs) | OpenAI hosted | Realtime voice in the TUI; Dictate button in the closed app | Composer text as `prompt` (removed TUI dictation); `keywords` in the current API | Supervised helper process; failure catalogue of lost speech |
| Handy | Desktop app | MIT | transcribe.cpp in a worker process | Pinned Hugging Face revisions, own R2 mirror, SHA-256 in the binary | Global shortcut, overlay, paste | Engine `vocabulary`, else Levenshtein plus Soundex | Worker process and the hash as trust anchor |
| VoiceInk | macOS app | GPL-3.0 | whisper.cpp, FluidAudio, transcribe.cpp, Apple, cloud | Whisper from Hugging Face `main` unhashed; newer paths pinned and hashed | Mini or notch recorder, paste | Vocabulary, phonetic matcher, Word Replacements, Auto Learn | Separate hints from replacements; learn narrowly |
| OpenWhispr | Electron app | MIT | Forked whisper-server sidecar, sherpa-onnx Parakeet | Hugging Face `main`, size only; GPU packs hashed | In chat mic swaps with Send; hotkey; paste | Prompt with tail placement, echo guard, learning from edits, agent tool | Echo guard and the agent tool contract |
| Whispering 7.11.0 | Tauri app | AGPL-3.0 | transcribe-rs in process | Own GitHub release for Parakeet; Whisper from Hugging Face; size only | cpal, MediaRecorder or FFmpeg recorders | Free text prompt; find and replace steps | Own release assets work as a model host |
| whisper-writer | Python app | GPL-3.0 | faster-whisper or OpenAI API | Hugging Face | Shortcut, four modes, types into the active window | Free text prompt | Skip the key click; unmaintained |
| Buzz | PyQt app | MIT | Bundled whisper-server and others | Hugging Face via `huggingface_hub` | File and live transcription | Initial prompt, replaced by rolling text in live mode on the in process backends | Do not let rolling context displace the vocabulary |

## Lessons that recur across projects

1. **Run the native engine in its own process.** whisper.cpp ends in `abort()` on any of its assertions (about 2,200 in core ggml and the CPU backend alone), sherpa-onnx calls `_Exit` on bad input, and GPU drivers fault. Handy, Codex and OpenWhispr all isolate native audio or inference in a child that can die alone.
2. **Pin every binary and model, and verify a hash compiled into the app.** Handy's catalog treats every host as an untrusted pipe. OpenWhispr, Whispering and VoiceInk's Whisper path check only size, or nothing. GitHub release assets serve range requests and work as a model host for files under 2 GiB, as sherpa-onnx, Whispering and OpenWhispr show.
3. **Never decode silence.** Text on silence appears across projects (phantom "Yeah" in Codex #36701, "Thank you for watching!" from whisper.cpp), and a dictionary makes it worse: recited glossaries (OpenWhispr), hotword loops (faster-whisper #1356), `PHD2, PHD2, PHD2` in Biorouter's benchmark. A VAD gate that returns nothing when there is no speech, plus an echo check against the exact prompt sent, is the common defence.
4. **A prompt holds about 50 gene symbols or acronyms, so choose them.** Count real tokens, select terms relevant to the current conversation, and put the most important last for whisper.cpp (it keeps the tail), or first for faster-whisper `hotwords` (it keeps the head). Better still, trim to budget yourself, as transcribe.cpp does. Research agrees that relevance beats volume past about 100 terms.
5. **Use two channels for the dictionary.** Recognition hints change what the model hears. Deterministic replacements from spoken or misheard forms to the written form fix what it wrote (VoiceInk, Talon, Superwhisper, AWS Transcribe). Run the replacements on every transcript, because biasing can silently switch off.
6. **Store the form the model emits.** Hotwords and replacements match tokens, so a written form like `TP53` cannot match "TP fifty three". Symbols made of letters and digits need spoken aliases and a normalized key match: `EPAS 1`, `EPAS-1` and `epas one` all reduce to `epas1`. Off the shelf fuzzy matchers skip such tokens.
7. **Learn narrowly, and let people undo it.** Shipped learners accept only small single substitutions, reject common words and inflections, cap suggestions, and never add a deleted word again. An app that inserts into its own composer can diff the inserted text against what was sent, with no accessibility permission.
8. **Give an agent tool deltas, not lists.** OpenWhispr's `update_dictionary(add, remove)` avoids the stale snapshot bug (#1295). VoiceInk and Wispr Flow keep learned entries as suggestions, with dismissals remembered.
9. **Capture PCM at 16 kHz mono with browser processing off.** Engines want raw PCM, and whisper.cpp needs FFmpeg to read WebM or Opus. Chromium's gain control changes the Windows system microphone volume. An AudioWorklet must load from a same origin file under a strict CSP.
10. **Design the button around lost speech.** Show an arming state until the first audio chunk arrives. Use one recording marker. Keep stop reachable. Insert at the caret. Never discard audio silently, and explain every refusal. Derive availability from what is installed. Give dictation one owning composer, and defer updates while recording.
11. **Prefer a final transcript to live partials at first.** Decoding a growing buffer again on every update costs a lot, and Whisper's 30 s window gives each partial a fixed encoder cost. Codex users asked to bring back a mode that records first and inserts one refined transcript (#56075). A later live preview can be replaced by a second offline pass on release.
12. **Read model licenses before rehosting.** MIT and Apache-2.0 need notices. CC-BY-4.0 needs attribution and a change note even for a GGUF conversion. The NVIDIA Open Model License needs a notice file. Avoid noncommercial and flow down terms for a default. Copy code only from MIT and Apache projects; GPL and AGPL apps (VoiceInk, Whispering, whisper-writer) give ideas only.
13. **Measure on the target speech.** The leaderboard changed protocol on 2026-11-02, vendor numbers use different protocols, and no published study covers dictated gene symbols. Biorouter's own measurements are in [speech model benchmarks](speech-model-benchmarks.md).

## Related documentation

- [Voice dictation research](README.md): the index of this folder.
- [Speech model benchmarks](speech-model-benchmarks.md): Biorouter's own measurements of engines, models and dictionary biasing.
- [Voice dictation](../../dictation/README.md): Biorouter's own dictation design, which this survey informs.
- [Dictation engine and models](../../dictation/engine-and-models.md): the engine, model catalog and hosting Biorouter chose from these options.
- [Dictation dictionary](../../dictation/dictionary.md): the dictionary design that applies the vocabulary lessons above.
- [Dictation surfaces](../../dictation/surfaces.md): where capture and transcription run on each Biorouter surface.
- [OpenAI Codex CLI review](../coding-agent-landscape/codex-cli.md): an earlier study of the same Codex repository's agent loop.
