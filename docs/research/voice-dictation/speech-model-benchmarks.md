# Speech model benchmarks

> **What this is.** The record of four measurement studies that compared local speech engines, speech models and dictionary biasing methods for Biorouter's planned dictation: the setup, material, metrics and results of each study, and what the numbers decide.
> **Status:** Current: measurements taken 2026-11-04; the material and versions are listed with each study
> **Audience:** developers building Biorouter's dictation engine, speech model catalog and personal dictionary, and maintainers choosing model tiers.

Biorouter's dictation plan transcribes speech on the person's own machine and steers the result toward a personal dictionary of rare terms such as UCSF, EPAS1 and natalizumab. Four studies, all run on 2026-11-04, measured how well that works. The prompt study measured whisper.cpp with the dictionary as a decoder prompt on synthetic speech. The engine study compared five runtimes and nine models on the same clips plus noisy and reverberant copies. The logit study prototyped a booster that raises the decoder's token scores for dictionary terms, up to 1,000 terms. The real speech study repeated the key experiments on recorded human speech. This document records each study so a later change can be measured against the same baseline, then states which design choices the numbers settle and which they leave open. The plan that applies these results is in [voice dictation](../../dictation/README.md).

## How to read these numbers

Every load bearing claim carries one of three labels.

| Label | Meaning |
|---|---|
| **[measured]** | Produced by a run in one of the four studies. Every table cell is measured unless it says otherwise. |
| **[code]** | Read in the source of a pinned upstream release or commit, listed in the setup. |
| **[inferred]** | Reasoning from measured or read facts, not tested. |

Four limits apply to everything below.

- **Most of the speech is synthetic.** Three studies used macOS `say` text to speech (TTS). TTS is clean, evenly paced and pronounces a term the same way every time. The relative effects (prompt against no prompt, VAD against no VAD) are the useful output. Absolute recall on real dictation is lower. The real speech study is the check, and it overturned three synthetic conclusions: that `small.en` matches turbo, that the correction pass makes no false edits, and that decoding with timestamps off is safe for short clips.
- **The samples are small.** The synthetic set has 81 target term occurrences per model and condition, so a difference of 2 to 4 points is noise. The two real speech sets have 85 and 78 occurrences, so 5 points is noise unless the paired test says otherwise.
- **One shared machine.** Every run used one Apple M4 Max that other agents were also using. The 1 minute load average ranged from 4 to 255 depending on the run. Latency figures are upper bounds for an idle M4 Max. No x86 machine was measured. A portable CPU build on 4 Apple cores stands in for Intel Mac, Windows and Linux.
- **Table labels.** Tables carry the letter of their study: P (prompt study), E (engine study), L (logit study), R (real speech study).

Terms used throughout:

| Term | Meaning |
|---|---|
| VAD | Voice activity detection: a small model (Silero here) that marks which parts of the audio contain speech. Used as a gate, it stops silence and noise from reaching the decoder. |
| Prompt | Text given to Whisper's decoder as if it were the transcript that came before. A comma separated list of dictionary terms is the usual form. |
| BPE | Byte pair encoding, the subword tokenizer Whisper was trained with. |
| Logit | The decoder's raw score for each possible next token. A logit bias adds to the score of chosen tokens. |
| Correction pass | A deterministic text pass after decoding that rewrites near misses to dictionary terms. |
| Echo | Dictionary text that appears in the transcript of audio with no speech in it. |
| Gene symbols | In the engine study, the pooled results for EPAS1, EGLN1, HIF-2α, PHD2 and VHL. |

## Shared setup

### Machine and toolchain

| Item | Value |
|---|---|
| Machine | Apple M4 Max, 16 cores (12 performance, 4 efficiency), 128 GB unified memory |
| OS | macOS 26.7.1 (25G241) |
| Toolchain | cmake 4.4.5, Apple clang 21.0.0 (clang-2100.4.1.1), Python 3.15.2 (standard library only, run with `python3 -I`) |
| Load | Shared with other agents throughout. Load average per study: prompt study up to 29; engine study 4.5 to 255 (latency pass 24 to 40); logit study 4 to 45; real speech study 4 to 116 |

### whisper.cpp build

All four studies used one copy of [whisper.cpp](https://github.com/ggml-org/whisper.cpp) at commit `4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad` (2026-11-03, version string 1.9.5-dev, ggml 0.28.0, MIT license). The Metal build:

```bash
cmake -B build -DCMAKE_BUILD_TYPE=Release -DGGML_METAL=ON -DWHISPER_BUILD_TESTS=OFF
cmake --build build -j 14 --config Release --target whisper-cli whisper-server
```

- Configure took 13 s and the build 33 s. The Metal and Accelerate BLAS backends were active, with the Metal library embedded.
- The default build is shared libraries: `whisper-cli` 847,320 B, `whisper-server` 1,250,096 B, libwhisper 432,464 B, libggml-metal 5,621,016 B; 9.5 MB in all. Shipping needs a static build or bundled libraries.
- The engine study added a CPU only copy built with `-DGGML_METAL=OFF -DGGML_NATIVE=OFF -DGGML_BLAS=OFF -DGGML_ACCELERATE=OFF` in 20.9 s. On arm64 the compiler default still enables dotprod and fp16 vector arithmetic (no SVE, i8mm or SME). This build is the proxy for CPU only machines.

### How whisper.cpp handles the prompt

These facts were read in the source at the commit above **[code]** and confirmed by measurement where noted.

| Behaviour | Detail |
|---|---|
| Prompt budget | `max_prompt_ctx = min(n_max_text_ctx, n_text_ctx / 2)`. Every Whisper model has `n_text_ctx = 448`, so the budget is 224 tokens, minus one for the special token that marks the start of the previous text: **223 prompt tokens**. |
| Over budget | Without `carry_initial_prompt`, the prompt joins the rolling history and only the **last** 223 tokens are kept. The front is dropped with no warning. **[measured]**: of a 164 term prompt, only the last 59 to 62 terms survived. |
| Temperature fallback | The prompt applies only while the decoding temperature is below 0.5. When decoding falls back to a temperature of 0.5 or more (typical on noise), the prompt is dropped for that attempt. |
| Separate calls | `whisper_full_default_params` sets `no_context = true`, so one utterance's text never leaks into the next call. |
| Tokenizer | `whisper_tokenize`, used for `initial_prompt` text and for `whisper-server`'s `prompt` field, does a greedy longest match over the vocabulary, not the ranked BPE merges the model was trained with. `prompt_tokens` takes token ids directly. |
| Server | `whisper-server` accepts the prompt only as text, sets no logits filter, and serializes inference behind one global mutex. Its default decoding is greedy with best of 2; `whisper-cli` defaults to beam search 5. |
| Integrated VAD | Silero runs inside `whisper_full`, the speech segments are joined with 0.1 s silences and decoded in one call. With no speech found, the call returns an empty result. |

### Model files

Every file was checked after download. The SHA-256 values equal the Hugging Face LFS object ids or the GitHub release asset digests, so an in house mirror can pin them directly.

| Engine | File | Bytes | SHA-256 | License | Source |
|---|---|---|---|---|---|
| whisper.cpp | `ggml-base.en.bin` | 147,964,211 | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` | MIT | [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) |
| whisper.cpp | `ggml-small.en.bin` (f16) | 487,614,201 | `c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d` | MIT | same |
| whisper.cpp | `ggml-small.en-q8_0.bin` | 264,477,561 | `67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067` | MIT | same |
| whisper.cpp | `ggml-small.en-q5_1.bin` | 190,098,681 | `bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30` | MIT | same |
| whisper.cpp | `ggml-medium.en-q5_0.bin` | 539,225,533 | `76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0` | MIT | same |
| whisper.cpp | `ggml-large-v3-turbo-q5_0.bin` | 574,041,195 | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` | MIT | same |
| whisper.cpp | `ggml-large-v3-turbo-q8_0.bin` | 874,188,075 | `317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1` | MIT | same |
| whisper.cpp VAD | `ggml-silero-v6.2.0.bin` | 885,098 | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` | MIT | [ggml-org/whisper-vad](https://huggingface.co/ggml-org/whisper-vad) |
| sherpa-onnx VAD | `silero_vad.onnx` | 643,854 | `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6` | MIT | k2-fsa `asr-models` release, 2025-07-11 |
| whisper.cpp Parakeet | `ggml-parakeet-tdt-0.6b-v3-q8_0.bin` | 668,757,119 | `4d64e9e96c2792186d072fde0034df0ad670cf680a2f53069052ead827fd600e` | CC-BY-4.0 (the card says MIT, but the base model `nvidia/parakeet-tdt-0.6b-v3` is CC-BY-4.0 and a conversion cannot relicense it) | ggml-org/parakeet-GGUF at `35156454` (2026-06-16); only v3 files exist there |
| llama.cpp | `Qwen3-ASR-0.6B-Q8_0.gguf` | 804,749,248 | `bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971` | Apache-2.0 (from `Qwen/Qwen3-ASR-0.6B`) | ggml-org/Qwen3-ASR-0.6B-GGUF at `928ab958` (2026-04-13) |
| llama.cpp | `mmproj-Qwen3-ASR-0.6B-Q8_0.gguf` | 214,392,480 | `41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d` | Apache-2.0 | same |
| llama.cpp | `Qwen3-ASR-1.7B-Q8_0.gguf` | 2,165,034,944 | `58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57` | Apache-2.0 | ggml-org/Qwen3-ASR-1.7B-GGUF at `36a67868` (2026-04-13). Over GitHub's 2 GiB per asset limit; only Q8_0 and bf16 are published |
| llama.cpp | `mmproj-Qwen3-ASR-1.7B-Q8_0.gguf` | 355,709,344 | `46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d` | Apache-2.0 | same |
| sherpa-onnx | `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2` | 482,468,385 | `157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad` | CC-BY-4.0 | k2-fsa/sherpa-onnx `asr-models` release (2025-08-16). Unpacked encoder 652,184,296 B, decoder 7,257,753 B, joiner 1,739,080 B |
| sherpa-onnx | `bpe.vocab` from the `.nemo` archive | 10,393 | `dbe7ec6bd066c4526080940e1614e6582b9161515e856c2c39a1d49267c88cbc` | CC-BY-4.0 | extracted from `nvidia/parakeet-tdt-0.6b-v2` with one 4 MB HTTP range request |
| transcribe.cpp | `Qwen3-ASR-0.6B-Q8_0.gguf` (a different conversion) | 850,423,456 | `f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8` | Apache-2.0 | handy-computer/Qwen3-ASR-0.6B-gguf at `7e7c69a5` (2026-10-16) |

### Engines other than whisper.cpp

| Engine | Version and file | License |
|---|---|---|
| llama.cpp `llama-server` | [release b11429](https://github.com/ggml-org/llama.cpp/releases/tag/b11429), `llama-b11429-bin-macos-arm64.tar.gz`, 11,971,406 B, SHA-256 `740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459`. This equals the digest Biorouter already pins for its bundled Llama Server. `--version` reports build 11429, commit d81235049. | MIT |
| [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) | 2.0.1 from PyPI (git `af2d8d81`, 2026-10-26), CPU provider, 4 threads. Its `libonnxruntime.dylib` is 29,006,384 B and `libsherpa-onnx-c-api.dylib` 4,139,368 B. | Apache-2.0 |
| [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) | v0.4.1 (2026-10-22), `transcribe-native-0.4.1-macos-arm64-metal.tar.gz`, 2,491,746 B, SHA-256 `7369e36dc2a5fc5d52eee5766fa03ad22d3f533c8688bf1deccf3ffdc202ea24`; Python binding wheel 47,639 B, SHA-256 `f373b6e20a4d897ed0dcaa2fe7fae183379777432b3a76641c42506c77b442ad` | MIT |
| whisper.cpp Parakeet | `parakeet-cli` and libparakeet 1.9.5 built in the same whisper.cpp tree | MIT |

## Metrics

The prompt study defined these, and the later studies reused its scorer unchanged, adding only what a new failure needed.

| Metric | Definition | Used in |
|---|---|---|
| Strict recall | The term's exact written form appears: case sensitive, at alphanumeric word boundaries, counted at most as often as the reference holds it per clip. In the real speech study a canonical form that is all lowercase (a generic drug name) is matched without case, because the corpus's own references capitalize generics inconsistently; brands, gene symbols and acronyms must match exactly. | all |
| Case insensitive recall | Strict recall with case ignored (catches `Natalizumab`, `Spoke`). | P, R |
| Lenient recall | A window of 1 to 4 tokens whose normalized key equals the term's key. The key is lowercase alphanumerics only, with α read as alpha and number words read as digits. It catches `HIF2 alpha`, `Bio Router`, `CRISPR-I`, `E-PAS-1`, `HIF2A`. | all |
| False insertion | A dictionary term found (leniently) in a speech transcript beyond the reference count. The engine study also counts any of the 150 distractor terms found strictly (none is ever spoken), and calls a transcript with six or more spurious terms a **runaway**. The logit study counts every other entry of the active dictionary strictly. | all |
| Probe insertion | Any dictionary term written into the transcript of an ordinary sentence that contains near homophones ("I spoke with", "PhD two years ago", "crisper drawer"). | P, L |
| Echo | The text produced for audio with no speech in it, flagged when it contains a dictionary term. The real speech study reports the share of nonspeech clips with any text (bracketed tags ignored) and the share with dictionary text. | all |
| WER | Word error rate: word level edit distance over lowercased tokens, with hyphens and slashes split and other punctuation removed. Case and hyphen errors do not count, so it is a rough measure. The real speech study also removes fillers. | all |
| RTF | Real time factor: total decode wall time divided by total audio duration. | P, E, R |
| Warm latency | Decode time per utterance with the model already loaded. The first clip of each run is decoded twice and the first call is reported separately as the cold first call. | all |
| Model load | Time to load the model file from a warm page cache. A true cold disk read was not measured. | P, E |
| Peak RSS | Maximum resident set size from `/usr/bin/time -l`, the maximum over every process of that run. The engine study also reports the peak memory footprint. | E |
| Collapse | An output with at most 25% of the words of a reference of 8 words or more. | R |
| False edit | A correction pass edit that writes a term the reference does not contain. | P, E, L, R |
| Flips per clip | Decoding steps where the logit bias changed the token the decoder chose. | L |
| Pooled | Clean, SNR 20 dB, SNR 10 dB and reverb conditions together: 324 target occurrences. Rows that ran only clean and SNR 10 (the two CPU only rows and the two transcribe.cpp rows in E1) pool those two: 162 occurrences in 84 utterances. Such a figure does not compare directly with a four condition one. | E |

## The synthetic dictation set

Three studies share one set of TTS clips.

- **Terms.** Fourteen targets, each with one written form counted as correct: `EPAS1, EGLN1, HIF-2α, UCSF, Baranzini, SPOKE, OMOP, natalizumab, ocrelizumab, scRNA-seq, CRISPRi, Biorouter, PHD2, VHL`. `HIF2A` counts only as lenient (an alias), `BioRouter` only as lenient (the brand is spelled with a lowercase r), and `spoke` only as lenient (the knowledge graph is SPOKE).
- **Sentences.** Twelve dictation style sentences carry the terms, every term twice (CRISPRi once), and two control sentences carry none. The text given to `say` spells terms the way a person says them. Three examples:

| Reference | Text given to `say` |
|---|---|
| Check whether EPAS1 variants are enriched in the Tibetan cohort. | Check whether E PAS one variants are enriched in the Tibetan cohort. |
| EGLN1 encodes PHD2, which hydroxylates HIF-2α under normal oxygen. | E G L N one encodes P H D two, which hydroxylates hif two alpha under normal oxygen. |
| Pull the OMOP tables for patients at UCSF who started natalizumab. | Pull the oh mop tables for patients at U C S F who started natalizumab. |

- **Voices.** Samantha (en_US), Daniel (en_GB) and Karen (en_AU): 42 clips of 3.5 to 6.0 s, 189 s in total, mean 4.5 s, 81 target occurrences per model and condition.
- **Nonspeech.** Two 6 s clips: digital silence, and pink noise at −39.7 dBFS RMS with a 60 Hz hum at −50 dBFS.
- **Long clips.** Two clips of 70 to 72 s join all 14 sentences of one voice with 0.6 s gaps.
- **Distractors.** 150 unrelated terms (60 gene symbols, 25 drugs, 40 methods and tools, 25 datasets and institutions) dilute the dictionary to 54 or 164 terms, with the 14 targets interleaved evenly.
- **Known artifacts.** `say` reads "seq" as the Latin abbreviation, so scRNA-seq is spoken as "S C R N A seek", and "crisper I screen" is sometimes heard as "crisper ice cream".

## Prompt biasing on synthetic speech

### Question

How much does a personal dictionary help whisper.cpp write rare biomedical terms correctly when it is given as a decoder prompt, with or without beam search, and when a correction pass runs after decoding? What does it cost in latency, and what does it do on silence, on noise and on long recordings?

### Setup

- whisper.cpp Metal build and machine as in [shared setup](#shared-setup); load average up to 29, run to run timing noise about ±20%.
- Models: `base.en`, `small.en` (f16), `medium.en-q5_0` and `large-v3-turbo-q5_0`, plus Silero VAD v6.2.0.
- A small C++ test program linked against the built libwhisper loads a model once and decodes every clip with the same parameters as `whisper-cli -nt -l en`: greedy with best of 5 (or beam 5), temperature increment 0.2, entropy threshold 2.4, log probability threshold −1, no speech threshold 0.6, flash attention on, Metal on, 8 threads. On one clip it printed text identical to `whisper-cli` with the same prompt.

### Material and its limits

The [synthetic dictation set](#the-synthetic-dictation-set). The correction pass thresholds and its stop list were written while looking at `base.en` output, with no held out set, so the corrected figures are optimistic. A text only probe of 25 ordinary sentences with near homophones partly offsets this.

### Method

Conditions, each decoded greedily unless noted:

| Condition | Prompt | Prompt tokens (`base.en` / turbo) |
|---|---|---|
| No prompt | none | 0 |
| Glossary | the 14 written forms as a comma separated list | 62 / 63 |
| Sentence | one natural sentence that uses all 14 terms | 77 / 79 |
| 54 terms | the 14 targets among 40 distractors, just under the budget | 209 / 217 |
| 164 terms | the 14 targets among 150 distractors, about three times the budget | 655 / 677 |
| Beam 5 | no prompt, glossary or sentence with beam search 5 | as above |
| VAD | no prompt, glossary or sentence with Silero v6.2.0 inside `whisper_full` | as above |

Fourteen terms cost about 62 tokens, about 4.4 per term with `", "` separators, measured with `whisper_tokenize` against each model's own vocabulary. With a budget of 223 tokens, a comma list holds about 45 to 50 short terms.

The correction pass tokenizes the transcript and considers every window of 1 to 4 tokens that does not cross punctuation. A window matches a term when its normalized key equals the term's key (an exact key match), or, for keys of 4 characters or more with a length ratio between 0.67 and 1.5 and matching digits, when the edit distance similarity of the keys is at least 0.8, or at least 0.6 together with a similarity of 0.75 or more between simplified Metaphone codes (a fuzzy match). Multi token fuzzy windows may not start or end with a stop word or consist only of English words. A **guard** refuses to rewrite a single token that is an English word (from `/usr/share/dict/words`, 235,976 entries) unless it is capitalized in mid sentence or has inner capitals.

### Results

**Table P1. Strict recall by model and prompt** (81 occurrences each, greedy unless noted) **[measured]**

| Model | Download | No prompt | Glossary | Sentence | 54 terms (fits) | 164 terms (truncated) | Glossary, beam 5 | Sentence, beam 5 |
|---|---|---|---|---|---|---|---|---|
| `base.en` | 148 MB | 27% | 72% | 77% | 63% | 35% | 78% | 84% |
| `small.en` | 488 MB | 36% | 90% | 93% | 83% | 58% | 93% | 94% |
| `medium.en-q5_0` | 539 MB | 47% | 86% | 86% | 84% | 67% | 86% | 86% |
| `large-v3-turbo-q5_0` | 574 MB | 43% | 84% | 81% | 74% | 54% | 86% | 84% |

**Table P2. Lenient recall and WER** **[measured]**

| Model | Lenient, no prompt | Lenient, glossary | WER, no prompt | WER, glossary | Lowest WER (condition) |
|---|---|---|---|---|---|
| `base.en` | 59% | 80% | 15.7% | 9.2% | 3.9% (sentence, beam 5) |
| `small.en` | 75% | 98% | 11.3% | 3.5% | 2.8% (sentence, beam 5) |
| `medium.en-q5_0` | 86% | 96% | 9.8% | 2.8% | 2.4% (glossary, beam 5) |
| `large-v3-turbo-q5_0` | 78% | 94% | 13.7% | 2.4% | 0.7% (sentence, beam 5) |

What P1 and P2 show:

- **The prompt is the largest lever:** +39 to +54 points of strict recall. Without a prompt most misses are format, not hearing (`PhD2`, `HIF2 alpha`, `CRISPR-I`, `SCRNA-seq`, `spoke`), since lenient recall is already 59% to 86%. The prompt also fixes real mishearings of drug names (`Natala Zoom app`, `Acralazumab`).
- **A sentence and a comma list were within noise of each other.** A comma list is easier to build from a dictionary and its token cost is predictable.
- **Beam 5 adds 0 to 7 points on top of a prompt** (largest on `base.en`). Against the glossary run it added 21% decode time on `base.en`, 17% on `medium.en-q5_0` and 23% on turbo, but 58% on `small.en` (105 to 166 ms; P6). Beam 5 without a prompt adds about 1 point.
- **Some priors resist the prompt.** `medium.en` and turbo wrote `BioRouter` in 6 of 6 cases under every prompt. `small.en` wrote `EPA's 1` or `EPA-1` for EPAS1 under the glossary (0 of 6 strict) but EPAS1 under the sentence (5 of 6). A prompt is a bias, not a constraint.
- **Size is not monotonic here.** `small.en` with a prompt matched or beat the two larger models on clean TTS, at a third of the decode time. The real speech study reverses this.
- **Dilution erodes the exact form first.** At 54 terms (inside the budget) strict recall fell 2 to 10 points while lenient recall barely moved. At 164 terms the front of the prompt was dropped and the dropped terms returned to their no prompt recall exactly: natalizumab on `base.en` went 0 of 6 with no prompt, 6 of 6 with the glossary, 0 of 6 at 164 terms.
- **VAD did not hurt speech.** With Silero on, strict recall stayed within ±2 points of the same prompt without VAD on every model (`base.en` 27, 73, 77; turbo sentence 81 to 83).

**Table P3. Output on the two nonspeech clips** **[measured]**

| Model | No VAD, no prompt (silence / noise) | No VAD, with a dictionary prompt: worst output | VAD on, every prompt |
|---|---|---|---|
| `base.en` | `you` / `[BLANK_AUDIO]` | `(music)`, `(water rushing)`, `STREET,` | empty / empty |
| `small.en` | `you` / `[BLANK_AUDIO]` | `PHD2, PHD2, PHD2` (glossary, noise); `PHD2, SPOKE, PHD2,` (glossary with beam 5, noise) | empty / empty |
| `medium.en-q5_0` | `you` / `[silence]` | `Thank you for watching!` | empty / empty |
| `large-v3-turbo-q5_0` | `you` / `.` | `FD2, VHL` (glossary with beam 5, both clips); `University of California,` (164 terms, silence) | empty / empty |

- Without VAD, Whisper wrote text on nearly every nonspeech decode, and 4 of the 64 decodes without VAD (4 models, 8 conditions, 2 clips) emitted dictionary terms outright: `small.en` with the glossary on noise, `small.en` with the glossary and beam 5 on noise, and turbo with the glossary and beam 5 on both clips.
- **With VAD all 24 nonspeech decodes were empty** (4 models, 3 prompts, 2 clips), returned in about 15 ms. The noise clip without VAD took about 1.3 s on the server, a temperature fallback loop.
- On the 42 speech clips, no condition on any model inserted a dictionary term that was not spoken: 0 insertions in all 44 model and condition cells.

**Table P4. Strict recall after the correction pass** (guard on / guard off) **[measured]**

| Model | No prompt | Corrected | Glossary | Corrected | Glossary, beam 5 | Corrected |
|---|---|---|---|---|---|---|
| `base.en` | 27% | 81% / 88% | 72% | 98% / 98% | 78% | 100% / 100% |
| `small.en` | 36% | 90% / 100% | 90% | 99% / 100% | 93% | 100% / 100% |
| `medium.en-q5_0` | 47% | 94% / 99% | 86% | 100% / 100% | 86% | 100% / 100% |
| `large-v3-turbo-q5_0` | 43% | 91% / 99% | 84% | 99% / 100% | 86% | 99% / 100% |

- On the 42 transcripts the pass made **0 false edits** and lowered WER every time (turbo with no prompt 13.7% to 2.8%; `small.en` with the glossary 3.5% to 0.4%).
- Most fixes were exact key matches (`BioRouter`, `pHD2,`, `HIF2-alpha`, `E-PAS-1`, `SCRNA-seq`). The rest were fuzzy (`ocralizumab`, `Natala Zoom app.`, `EPS-1`, `SCRNASIC`, `Buranzini`, and `CRISPR` to `CRISPRi`).
- **On 25 ordinary sentences with near homophones, every edit is wrong.** Guard on: 2 edits (`PhD two` to `PHD2` by exact key; `spokes` to `SPOKE` by fuzzy match). Guard off: 7 edits (`spoke` to `SPOKE` three times, `crisper` and `crisp` to `CRISPRi`, plus the two above).
- `CRISPR` to `CRISPRi` is correct here but would be wrong for a person who says CRISPR: the pass extends a correctly spelled real term into a longer dictionary term.

**Table P5. Long recordings: sentences kept out of 14** (glossary prompt; Samantha / Daniel) **[measured]**

| Model | Timestamps off (`no_timestamps`) | Timestamps on | VAD, timestamps off |
|---|---|---|---|
| `base.en` | 14 / 13 | 14 / 14 | 14 / 13 |
| `small.en` | 14 / 14 | 14 / 14 | 14 / 14 |
| `medium.en-q5_0` | 7 / 8 | 13 / 14 | 12 / 14 |
| `large-v3-turbo-q5_0` | 13 / 13 | 14 / 14 | 14 / 14 |

- With timestamps off, whisper.cpp dropped whole sentences at the 30 s window boundaries: `medium.en` kept 7 to 8 of 14 (WER 47% to 60%) with or without a prompt, and `base.en` with no prompt kept 3 of 14 for one voice (WER 81%).
- With timestamps on, 12 to 14 of 14 survived on every model, and the glossary still helped (turbo WER 9.2% with no prompt, 0% to 2% with the glossary).
- `carry_initial_prompt` changed little: 0 to +2 strict hits in most rows, −1 once, +7 once.

**Table P6. Latency per 3.5 to 6 s utterance, M4 Max, Metal, 8 threads** **[measured]**

| Model | Greedy, no prompt | Glossary | Glossary, beam 5 | RTF (greedy) | Model load |
|---|---|---|---|---|---|
| `base.en` | 39 ms | 43 ms | 52 ms | 0.009 | about 100 to 120 ms |
| `small.en` | 93 ms | 105 ms | 166 ms | 0.021 to 0.023 | about 180 to 450 ms |
| `medium.en-q5_0` | 271 ms | 296 ms | 347 ms | 0.060 to 0.066 | about 220 to 500 ms |
| `large-v3-turbo-q5_0` | 269 ms | 274 ms | 337 ms | 0.060 to 0.061 | about 200 to 450 ms |

- **The prompt is nearly free:** +2% to +13% decode time; `whisper-cli` reported 5.9 ms for 68 prompt tokens on turbo. Each call still encodes a padded 30 s window, which dominates the cost.
- **A warm `whisper-server` request** with the glossary, JSON output and timestamps off took 0.10 to 0.13 s (`small.en`) and 0.27 to 0.29 s (turbo) including the HTTP round trip. VAD added 15 to 25 ms. The server answered its health check 0.26 s and 0.37 s after spawn; RSS after the requests was 809 MB and 828 MB.
- **A new process per utterance** took 0.20 to 0.22 s (`base.en`) and 0.53 to 0.57 s (turbo), mostly load and Metal setup. The very first launch of a freshly built binary took about 2.5 to 3 s (first model load 2,520 ms), which looks like Metal shader compilation; later processes did not pay it.

### What the evidence does not show

- Anything about real speech: accents, disfluency, laptop microphones, room noise.
- CPU only decoding, Intel Macs, or slower Apple Silicon (M1 or M2 base); expect several times these latencies there **[inferred]**.
- Correction pass false edit rates on held out text; the thresholds were tuned on this data.
- Whether a sentence beats a comma list; the difference was within noise.

## Engine and model head to head

### Question

On the same clips and scorer, plus noisy and reverberant copies, which engine, model and biasing method should Biorouter use? Candidates were whisper.cpp with a prompt, Qwen3-ASR on a second `llama-server` with the dictionary as context, NVIDIA Parakeet on sherpa-onnx with hotwords, whisper.cpp's own Parakeet port, and transcribe.cpp. The study also asked how many terms each method can carry, what each gets wrong, and what each costs in latency, memory and download size.

### Setup

Machine, whisper.cpp builds, engines and model files as in [shared setup](#shared-setup). The load average ranged from 4.5 to 255 across the matrix runs; a separate sequential latency pass ran at 24 to 40 and is the latency to quote.

| Arm | Engine and settings | Models | How the dictionary reaches the decoder | VAD |
|---|---|---|---|---|
| whisper.cpp | Metal, 8 threads, greedy (best of 5), timestamps off; the prompt study's test program | `small.en` f16, q8_0, q5_1; `large-v3-turbo` q5_0, q8_0 | `initial_prompt` text; later run: the same list as BPE token ids through `prompt_tokens` | off and on (inside `whisper_full`) |
| whisper.cpp, CPU only | portable build, 4 threads | `small.en-q8_0`, `large-v3-turbo-q5_0` | `initial_prompt` text | off and on |
| whisper.cpp Parakeet | `libparakeet`, greedy TDT decoding, Metal 8 threads and CPU 4 threads | Parakeet TDT 0.6B v3 q8_0 | none: the API has no prompt or hotword field **[code]** | external gate |
| Qwen3-ASR | `llama-server` b11429 with `--mmproj`, `--parallel 1`, `--api-key`, `-c 8192`, `-ngl 99` on 127.0.0.1; temperature 0, at most 256 tokens | Qwen3-ASR 0.6B Q8_0, 1.7B Q8_0 | (1) system message on `/v1/chat/completions` with an audio only user turn, Qwen's own layout; (2) the same list space joined; (3) the `prompt` field of `/v1/audio/transcriptions`, which replaces llama.cpp's default instruction; (4) layout 1 with the reply prefilled with `language English<asr_text>` to force the language | external gate |
| sherpa-onnx | CPU, 4 threads, `modified_beam_search` with 4 active paths, `modeling_unit="bpe"`, the real `bpe.vocab`, a hotword list per stream | Parakeet TDT 0.6B v2 int8 | hotwords (a context graph) at score 1.5 and 2.5, as written forms only or written forms plus spoken aliases | off and on (sherpa's Silero, segments joined with no padding) |
| transcribe.cpp | prebuilt Metal library and Python binding | Qwen3-ASR 0.6B Q8_0 (its own conversion); the same turbo q5_0 file | `vocabulary` list (space joined for Qwen; comma joined for Whisper; drops terms from the end when over budget) | external gate; English forced, plus one run with automatic language |

### Material and its limits

- The [synthetic dictation set](#the-synthetic-dictation-set), copied unchanged, scored with a copy of the prompt study's scorer. Run on the prompt study's own transcripts, it reproduced that study's tables exactly.
- **Degraded copies** of every speech clip, made with a standard library script: pink noise (seeded per clip) mixed at exactly 20.00 dB and 10.00 dB signal to noise ratio (SNR), with speech power measured over active 20 ms frames within 40 dB of the loudest frame; and a light synthetic room (four damped comb filters and two allpass filters, RT60 about 0.4 s, 10 ms delay before the first reflection, direct to reverberant ratio 6.5 to 6.7 dB).
- Silero v6.2.0 with its default parameters found exactly one speech segment in every speech clip in all four conditions: 168 clip instances (42 clips in each of clean, SNR 20, SNR 10 and reverb), of which 126 are degraded. It found none in either nonspeech clip; the nonspeech clips were checked in the clean pass only. Over all four conditions it took 5.6 to 18.3 ms per clip, median 8.5 ms (the clean pass alone took 7 to 12 ms). A VAD gate never dropped a degraded utterance.
- **Spoken aliases** were written by hand for the 14 targets the way a person would type "how I say it" (`E PAS one` for EPAS1, `oh mop` for OMOP, `U C S F`, `bio router`, `PHD two`). SPOKE, Baranzini and the two drug names got none. Every distractor got rule generated aliases (`T P fifty three` for TP53). The 14, 54 and 164 term lists became 31, 110 and 254 hotword phrases.
- Limits: still synthetic (three TTS voices, stationary noise, a simulated room); transcribe.cpp ran on two conditions only (clean and SNR 10); the CPU proxy is ARM, not x86.

### Method

Each arm ran with no dictionary and with the 14, 54 and 164 term lists, with VAD on and off where the engine supports it, then through two correction variants: a deterministic replacement of spoken aliases with written forms followed by the prompt study's correction pass over the 14 targets, and the same pass over the arm's whole dictionary to expose collisions. All metrics are as defined in [metrics](#metrics). Gene symbols pool EPAS1, EGLN1, HIF-2α, PHD2 and VHL (30 per condition, 120 pooled).

### Results

**Table E1. Decision table: strict recall by condition, pooled recall, and cost** **[measured]**

Cells are percentages. "Spurious" counts utterances that gained a dictionary term never spoken. Whisper rows are VAD on runs. "n/a" means not run. A row with "n/a" under SNR 20 and reverb pools clean and SNR 10 only (84 utterances, "of 84" in the spurious column), so its pooled figure does not compare directly with a four condition row.

| Engine and model | Dictionary channel, size | Clean | SNR 20 | SNR 10 | Reverb | Pooled raw → corrected | Gene symbols raw | Spurious | Warm ms per utterance | Peak RSS | Download |
|---|---|---|---|---|---|---|---|---|---|---|---|
| whisper.cpp Metal, `small.en-q5_1` | none | 42 | 36 | 31 | 37 | 36 → 84 | 50 | 0 of 168 | 130 | 0.6 GB | 190 MB |
| whisper.cpp Metal, `small.en-q5_1` | prompt text, 14 | 91 | 85 | 77 | 88 | 85 → 95 | 77 | 0 of 168 | 130 | 0.6 GB | 190 MB |
| whisper.cpp Metal, `small.en-q5_1` | **prompt BPE ids, 14** | 94 | 91 | 84 | 91 | 90 → 94 | 98 | 1 of 168 | 130 | 0.6 GB | 190 MB |
| whisper.cpp Metal, `small.en-q5_1` | prompt BPE ids, 54 | 88 | 79 | 78 | 81 | 81 → 90 | 92 | 3 of 168 | 130 | 0.6 GB | 190 MB |
| whisper.cpp Metal, `small.en-q8_0` | prompt text, 14 | 91 | 84 | 79 | 84 | 85 → 96 | 72 | 0 of 168 | 127 | 0.7 GB | 264 MB |
| whisper.cpp Metal, `small.en` f16 | prompt text, 14 | 90 | 83 | 78 | 84 | 84 → 95 | 71 | 0 of 168 | 107 | 0.9 GB | 488 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | none | 43 | 40 | 32 | 40 | 39 → 90 | 36 | 0 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | prompt text, 14 | 84 | 80 | 78 | 79 | 80 → 98 | 92 | 0 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | **prompt BPE ids, 14** | 94 | 94 | 89 | 94 | 93 → 99 | 97 | 2 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | prompt text, 54 | 72 | 68 | 62 | 72 | 68 → 93 | 78 | 0 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | **prompt BPE ids, 54** | 88 | 86 | 78 | 85 | 84 → 95 | 98 | 0 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q5_0` | prompt text, 164 (truncated) | 54 | 52 | 44 | 51 | 50 → 91 | 59 | 1 of 168 | 275 | 0.9 GB | 574 MB |
| whisper.cpp Metal, `large-v3-turbo-q8_0` | prompt text, 14 | 83 | 80 | 75 | 80 | 80 → 97 | 93 | 0 of 168 | 262 | 1.3 GB | 874 MB |
| whisper.cpp CPU 4 threads, `small.en-q8_0` | prompt text, 14 | 90 | n/a | 79 | n/a | 85 → 97 | 72 | 0 of 84 | 732 | 0.5 GB | 264 MB |
| whisper.cpp CPU 4 threads, `large-v3-turbo-q5_0` | prompt text, 14 | 84 | n/a | 78 | n/a | 81 → 97 | 95 | 0 of 84 | 9,633 | 0.8 GB | 574 MB |
| transcribe.cpp Metal, turbo q5_0 | `vocabulary`, 14 | 94 | n/a | 93 | n/a | 93 → 99 | 98 | 0 of 84 | 628 (matrix run) | 0.8 GB | 574 MB |
| whisper.cpp Parakeet Metal, TDT v3 q8_0 | none possible | 44 | 41 | 36 | 37 | 40 → 87 | 54 | 2 of 168 | 47 | 0.8 GB | 669 MB |
| sherpa-onnx CPU 4 threads, Parakeet TDT v2 int8 | none (beam 4) | 49 | 38 | 31 | 35 | 38 → 82 | 61 | 0 of 168 | 114 | 2.0 GB (Python process) | 482 MB |
| same | hotwords 1.5, 14 written | 91 | 83 | 74 | 78 | 81 → 97 | 84 | 0 of 168 | 123 | 2.0 GB | 482 MB |
| same | **hotwords 2.5, 14 written** | 93 | 88 | 83 | 89 | 88 → 98 | 89 | 0 of 168 | 123 | 2.0 GB | 482 MB |
| same | hotwords 2.5, 14 written plus aliases | 79 | 75 | 68 | 75 | 74 → 99 | 77 | 0 of 168 | 123 | 2.0 GB | 482 MB |
| same | hotwords 2.5, 164 written | 91 | 83 | 74 | 88 | 84 → 97 | 87 | 9 of 168 | 123 | 2.0 GB | 482 MB |
| `llama-server` b11429, Qwen3-ASR-0.6B Q8_0 | none | 40 | 30 | 23 | 31 | 31 → 71 | 46 | 0 of 168 | 285 | 2.1 GB | 1,019 MB |
| same | system message, 14 | 83 | 75 | 78 | 85 | 80 → 94 | 85 | 2 of 168 | 285 | 2.1 GB | 1,019 MB |
| same | transcription `prompt`, 14 | 73 | 72 | 74 | 72 | 73 → 96 | 76 | 0 of 168 | 243 (matrix run) | 2.1 GB | 1,019 MB |
| same | system message, 164 | 62 | 56 | 47 | 58 | 56 → 89 | 65 | 5 of 168 (4 runaways) | 285 | 2.1 GB | 1,019 MB |
| `llama-server` b11429, Qwen3-ASR-1.7B Q8_0 | system message, 14 | 88 | 68 | 67 | 86 | 77 → 97 | 75 | 1 of 168 | 277 | 3.6 GB | 2,521 MB |
| same | **transcription `prompt`, 14** | 88 | 84 | 83 | 88 | 85 → 99 | 95 | 0 of 168 | 590 (matrix run) | 3.6 GB | 2,521 MB |
| same | transcription `prompt`, 54 | 85 | 86 | 79 | 84 | 84 → 95 | 100 | 0 of 168 | 707 (matrix run) | 3.6 GB | 2,521 MB |
| same | transcription `prompt`, 164 | 79 | 78 | 69 | 78 | 76 → 91 | 94 | 16 of 168 | 745 (matrix run) | 3.6 GB | 2,521 MB |
| transcribe.cpp Metal, Qwen3-ASR-0.6B Q8_0 | `vocabulary`, 14, language automatic | 81 | n/a | 80 | n/a | 81 → 92 | 80 | 5 of 84 | 110 | 0.4 GB | 850 MB |

Notes on E1:

- The `llama-server` RSS figures are with `--cache-ram 0`. With the default 8,192 MiB host prompt cache the same servers reached 12.0 GB and 13.5 GB.
- Download sizes are model files only. The engines add about 9.5 MB of libraries for whisper.cpp on macOS, nothing new for `llama-server` (already bundled), about 33 MB for sherpa-onnx (onnxruntime 29 MB plus its C API 4 MB), and 2.5 MB for transcribe.cpp.

**Table E2. whisper.cpp model size and quantization** (14 term text prompt, VAD on) **[measured]**

| Model | Download | Clean | SNR 20 | SNR 10 | Reverb | Pooled raw → corrected | Gene symbols raw → corrected | WER raw → corrected |
|---|---|---|---|---|---|---|---|---|
| `small.en-q5_1` | 190 MB | 91 | 85 | 77 | 88 | 85 → 95 | 77 → 97 | 5.9 → 2.9 |
| `small.en-q8_0` | 264 MB | 91 | 84 | 79 | 84 | 85 → 96 | 72 → 98 | 6.4 → 3.0 |
| `small.en` f16 | 488 MB | 90 | 83 | 78 | 84 | 84 → 95 | 71 → 98 | 6.6 → 3.0 |
| `large-v3-turbo-q5_0` | 574 MB | 84 | 80 | 78 | 79 | 80 → 98 | 92 → 98 | 3.6 → 2.0 |
| `large-v3-turbo-q8_0` | 874 MB | 83 | 80 | 75 | 80 | 80 → 97 | 93 → 99 | 3.5 → 1.8 |

- **Quantization costs no measurable accuracy.** `small.en` q5_1 (39% of the f16 size) and q8_0 match f16 within one point in every condition, and turbo q5_0 matches q8_0. The clean results reproduce the prompt study (`small.en` 90%, turbo 84%). This says nothing about CPU speed: the quantization type changes CPU decode time (see the note under [model tiers per hardware class](#model-tiers-per-hardware-class)).
- **`small.en` leads on strict recall only through `Biorouter`,** which turbo writes `BioRouter` in 24 of 24 cases. Turbo is better on gene symbols (92% to 93% against 71% to 77%) and on WER. Under the 14 term prompt every `small.en` build wrote EPAS1 correctly in 0 of 24 cases (it wrote `EPA's 1`), against 15 to 17 of 24 for turbo.
- **Noise hurts `small.en` more.** From clean to SNR 10, `small.en` lost 12 to 14 strict points and turbo 3 to 8.

**Table E3. Why the prompt's tokenization matters** (turbo q5_0 and `small.en-q5_1`, VAD on, pooled) **[measured]**

transcribe.cpp scored 93% with the same turbo weights, pooled over clean and SNR 10 only (84 utterances). whisper.cpp's stock text prompt scored 80% pooled over all four conditions, and 84% (clean) and 78% (SNR 10) on the two conditions transcribe.cpp ran (E1). Three later runs on whisper.cpp isolated the cause.

| whisper.cpp change | Turbo, 14 terms | Turbo gene symbols | `small.en-q5_1`, 14 terms | Turbo, 54 terms |
|---|---|---|---|---|
| Stock `initial_prompt` text, timestamps off | 80% | 92% | 85% | 68% |
| Timestamps on, as transcribe.cpp decodes by default | 80% (3 conditions) | 91% | 86% | 68% |
| A leading space before the list | 77% | 88% | n/a | n/a |
| **The same list as `prompt_tokens`, tokenized with real BPE merges** | **93%** | **97%** | **90%** (gene symbols 77% → 98%) | **84%** (gene symbols 78% → 98%) |

- **The cause is the prompt tokenizer [code].** `whisper_tokenize` splits greedily, so the prompt's token sequence differs from the one the model would emit, exactly on the terms that improved: greedy ` Bio|rou|ter` against BPE ` B|ior|outer`; ` nat|ali|zu|ma|b` against ` nat|al|iz|um|ab`; on the English vocabulary `EPA|S|1` against `EP|AS|1`, which is why `small.en` wrote `EPA's 1`. transcribe.cpp tokenizes the prompt the way the model was trained.
- **The real BPE comes from the model's own vocabulary:** `gpt2.tiktoken` for `.en` models and `multilingual.tiktoken` for turbo, both MIT, shipped with OpenAI's Whisper.
- **The stronger prompt is also stronger on hard audio.** With BPE ids, turbo turned `uses` into `UCSF,` in 2 reverberant clips, and `small.en` at SNR 10 replaced one whole sentence with `Biorouter, PHD2, SPOKE, OMOP,`. The VAD gate kept nonspeech empty.
- `whisper-server`'s `prompt` field is text and goes through the greedy tokenizer, so a stock server cannot deliver this **[code]**.

**Table E4. How many terms each method carries** (pooled strict recall of the 14 targets, raw → corrected) **[measured]**

| Method | 14 terms | 54 terms | 164 terms | Spurious terms on speech |
|---|---|---|---|---|
| whisper.cpp `small.en-q8_0`, prompt text | 85 → 96 | 79 → 93 | 55 → 91 (truncated) | 2 to 3 clips at 54 (`Check whether` → `CHEK2, whether`; `Run a` → `RUNX1, run a`); 0 at 164 |
| whisper.cpp turbo q5_0, prompt text | 81 → 99 | 69 → 93 | 51 → 91 (truncated) | 1 clip at 164 |
| whisper.cpp turbo q5_0, prompt BPE ids | 93 → 99 | 84 → 95 | not run | 0 at 54 |
| whisper.cpp `small.en-q5_1`, prompt BPE ids | 90 → 94 | 81 → 90 | not run | 3 clips at 54 |
| transcribe.cpp turbo, `vocabulary` (clean and SNR 10 only) | 93 | 81 | 57 | 0 of 84 |
| Qwen3-ASR-0.6B, system message | 80 → 94 | 67 → 94 | 56 → 89 | 2 clips (1 runaway) at 54; 5 clips (4 runaways) at 164 |
| Qwen3-ASR-0.6B, transcription `prompt` | 73 → 96 | 62 → 94 | 52 → 91 | 1 clip at 54; 0 at 164 |
| Qwen3-ASR-1.7B, system message | 77 → 97 | 81 → 94 | 72 → 89 | 17 clips at 164, no runaway |
| Qwen3-ASR-1.7B, transcription `prompt` | 85 → 99 | 84 → 95 | 76 → 91 | 16 clips at 164 |
| sherpa-onnx hotwords 1.5, written forms | 81 → 97 | 79 → 98 | 79 → 96 | 10 clips at 164 (`started Nataliz UMAP`) |
| sherpa-onnx hotwords 2.5, written forms | 88 → 98 | 85 → 96 | 84 → 97 | 9 clips, 14 terms at 164 |

- **Whisper's prompt is a mechanism of about 45 terms.** BPE ids recover most of the loss up to the window size, but beyond 223 tokens truncation still applies.
- **Qwen3-ASR-1.7B's context is larger:** at 54 terms it holds recall and its gene symbols rise to 98% to 100%.
- **Qwen3-ASR-0.6B cannot carry a long list:** at 164 terms, 4 of 168 transcripts became dumps of the dictionary (`Check whether TP53, BRCA1, BRCA2, KRAS, EGFR, …` until the token limit).
- **Hotwords keep recall at 164 terms but capture nearby words:** natalizumab became `Nataliz UMAP` in 7 of 12 clips of one sentence at SNR 10 or better, and `map ocrelizumab` became `MAPT`.
- **The correction pass does not scale either:** with all 164 terms as targets it rewrote `encodes` to `ENCODE` 12 times on turbo's transcripts.

**Table E5. Gene symbol strict recall** (EPAS1, EGLN1, HIF-2α, PHD2, VHL; 120 occurrences, raw) **[measured]**

| Dictionary | Turbo q5_0, prompt text | Turbo q5_0, BPE ids | Turbo q8_0 | `small.en-q5_1`, text | `small.en-q5_1`, BPE ids | Qwen3-ASR-0.6B, system | Qwen3-ASR-1.7B, system | Qwen3-ASR-1.7B, transcription prompt | Hotwords 2.5 |
|---|---|---|---|---|---|---|---|---|---|
| 14 terms | 93 | **97** | 92 | 76 | **98** | 85 | 75 | 95 | 89 |
| 54 terms | 81 | **98** | 85 | 75 | 92 | 71 | **98** | **100** | 88 |
| 164 terms | 61 | not run | 58 | 64 | not run | 65 | **90** | **94** | 87 |
| After correction, 14 terms | 100 | 99 | 99 | 98 | 98 | 99 | 99 | 99 | 97 |

With BPE ids, whisper.cpp matches or beats Qwen3-ASR on gene symbols up to 54 terms. Qwen3-ASR-1.7B keeps an edge only beyond Whisper's 223 token window. In the chat layout the 1.7B often writes numbers as words (`EGLN one encodes PHD two`), which strict recall rejects and the correction pass fixes.

**Table E6. Nonspeech output and what removes it** **[measured]**

| Engine | Dictionary text in nonspeech output without a gate | With a gate |
|---|---|---|
| whisper.cpp prompt | `PHD2, PHD2, PHD2` (`small.en` q8_0 and f16, noise); `VHL, VHL, VHL` and `FD2, VHL` (turbo q8_0); `University of Michigan, NIH, California…` (turbo, 164 terms) | Silero inside `whisper_full`: **40 of 40 nonspeech decodes empty** (5 models, 4 dictionaries, 2 clips), speech recall unchanged within ±2 points |
| Qwen3-ASR, language detected automatically | 0.6B: none in 18 decodes (it answers `language None`). 1.7B: the transcription `prompt` with 14 terms wrote the whole dictionary on silence | external gate: Silero found no speech in either clip |
| Qwen3-ASR, language forced to English | always: both sizes, both runtimes, both clips (`EPAS1, EGLN1, HIF-2α, UCSF, Baranzini, …`) | external gate |
| sherpa-onnx hotwords | written forms at 1.5: none. With aliases: `Uh oh, oh, oh, oh, …` (the `oh mop` alias starts with an interjection). 164 terms at 2.5: `Mendelian randomized influence in the …` | sherpa's Silero gate: none |
| whisper.cpp Parakeet (no biasing) | none | not needed |
| transcribe.cpp Whisper turbo | `Glossary, APAS1`, `GR GR GR GR …` | external gate |

- Every engine wrote dictionary text from silence or noise in some configuration. A gate removed all of it.
- **Forcing Qwen3-ASR's language removes its own nonspeech answer** and turns it into a dictionary echo every time.
- **A VAD that trims tightly costs first words.** sherpa's VAD, run with no padding, cut `Loss of VHL` to `of VHL` and `Pull the OMOP` to `the OMOP` (WER 14.2% to 16.6% with no dictionary) and lowered recall 0 to 4 points. whisper.cpp's integrated VAD with its 30 ms padding cost nothing.

False insertions on speech differ by method **[measured]**:

- **whisper.cpp prompt:** none at 14 terms (0 of 168 on every model and condition). At 54 terms, 1 to 3 clips where an ordinary word that sounds like a listed symbol became that symbol.
- **Qwen3-ASR context: one listed term substituted for another, even at 14 terms.** `EGLN1 encodes PHD2` became `EPAS1 encodes PHD2`; `target EPAS1` became `target EGLN1`; `uses spoke` became `uses scRNA-seq`. The 0.6B did this in 2 clips with commas, 11 with spaces and 5 of 84 through transcribe.cpp; the 1.7B in 1. No other engine swapped one listed term for another. For gene symbols this is the worst error type, because the output looks right and means something else.
- **Hotwords:** none at 14 terms, distractor capture at 164.

Spoken aliases and the correction pass **[measured]**:

- The alias replacement plus the correction pass lifted most 14 term arms to 94% to 99% and every arm with no dictionary to 71% to 90%. It inserted a target in 11 of more than 300 arm and condition cells, one clip each, always by completing a word the engine had already got wrong (`uses` heard as `UCS` became `UCSF`).
- **Aliases in a hotword list made the model write spoken forms** (`E PAS one`, `HIF two alpha`, `bio router`): raw strict recall fell from 88% to 74%, and only the replacement pass brought it back (98% to 99%).
- **Aliases collide.** On 8 trap sentences the alias pass rewrote `Oh mop it up` to `OMOP`, `the bio router in the closet` to `Biorouter` and `CRISPR i think` to `CRISPRi`. On the 25 probe sentences it rewrote `PhD two years ago` to `PHD2`, because alias matching ignored case.

**Table E7. Latency, sequential pass** (clean clips, 14 term dictionary where the engine takes one; load average 24 to 40) **[measured]**

| Engine and model | Warm mean | Median | 90th percentile | ms per second of audio | First call | Model load | Nonspeech clips without a gate |
|---|---|---|---|---|---|---|---|
| whisper.cpp Metal, `small.en` f16 | 107 ms | 106 | 120 | 23.9 | 113 ms | 357 ms | 2,751 / 1,135 ms |
| whisper.cpp Metal, `small.en-q8_0` | 127 ms | 124 | 145 | 28.3 | 129 ms | 218 ms | 3,509 / 172 ms |
| whisper.cpp Metal, `small.en-q5_1` | 130 ms | 128 | 149 | 29.1 | 369 ms | 304 ms | 2,389 / 1,002 ms |
| whisper.cpp Metal, turbo q5_0 | 275 ms | 274 | 281 | 61.3 | 277 ms | 230 ms | 1,551 / 1,395 ms |
| whisper.cpp Metal, turbo q8_0 | 262 ms | 261 | 269 | 58.5 | 261 ms | 319 ms | 744 / 1,336 ms |
| whisper.cpp CPU 4 threads, `small.en-q8_0` | 726 to 745 ms | 722 to 742 | n/a | about 162 | 712 to 929 ms | 176 to 250 ms | n/a |
| whisper.cpp CPU 4 threads, turbo q5_0 | 9,033 to 9,633 ms | 8,838 to 8,967 | n/a | about 2,050 | 8,774 to 8,860 ms | 124 to 174 ms | n/a |
| whisper.cpp Parakeet Metal, TDT v3 | 47 ms | 45 | 50 | 10.5 | 250 ms | 250 ms | 97 / 38 ms |
| whisper.cpp Parakeet CPU 4 threads | 125 ms | 115 | 148 | 27.9 | 113 ms | 454 ms | 157 / 166 ms |
| sherpa-onnx CPU 4 threads, beam 4, 14 hotwords | 123 ms | 119 | 142 | 27.4 | 131 ms | 980 ms | 168 / 166 ms |
| `llama-server`, Qwen3-ASR-0.6B (HTTP, chat route) | 285 ms | 270 | 383 | 63.6 | 429 ms | 1.45 s spawn to healthy | 142 / 101 ms |
| `llama-server`, Qwen3-ASR-1.7B (same) | 277 ms | 266 | 340 | 61.7 | 296 ms | 1.93 s | 96 / 107 ms |
| transcribe.cpp Metal, Qwen3-ASR-0.6B | 110 ms | 107 | 133 | 24.4 | 123 ms | 240 ms | 57 / 63 ms |

- **Whisper's cost is flat per 30 s window,** so a 4 s and a 25 s utterance cost about the same. Parakeet and Qwen3-ASR scale with audio length.
- **Ungated nonspeech costs whisper.cpp 0.7 to 3.5 s** in temperature fallback loops; the VAD gate returns in about 10 ms.
- **Turbo is not a CPU model:** 9 to 9.6 s per 4 to 6 s utterance on the 4 thread portable build (RTF about 2.1).
- Hotwords added no measurable decode time. Building the per request hotword graph took a median 0.18 ms for 14 terms and 0.85 ms for 164 terms.
- The prompt's own cost could not be separated from load noise here; the prompt study measured +2% to +13% on a quieter machine.

**Table E8. Peak memory** (`/usr/bin/time -l`) **[measured]**

| Engine and model | Max RSS | Peak memory footprint |
|---|---|---|
| whisper.cpp Metal, `small.en` q5_1 / q8_0 / f16 | 595 / 686 / 948 MB | 717 / 808 / 1,070 MB |
| whisper.cpp Metal, turbo q5_0 / q8_0 | 937 / 1,257 MB | 1,059 / 1,380 MB |
| whisper.cpp CPU, `small.en-q8_0` / turbo q5_0 | 585 / 860 MB | 584 / 860 MB |
| whisper.cpp Parakeet v3, Metal / CPU | 797 / 744 MB | 936 / 743 MB |
| sherpa-onnx Parakeet v2 int8 (whole Python process) | 1,991 to 2,007 MB | 1,970 to 1,986 MB |
| `llama-server` Qwen3-ASR-0.6B, defaults / `--cache-ram 0` | 12,030 / 2,136 MB | 11,353 / 1,454 MB |
| `llama-server` Qwen3-ASR-1.7B, defaults / `--cache-ram 0` | 13,518 / 3,626 MB | 11,505 / 1,637 MB |
| transcribe.cpp, Qwen3-ASR-0.6B / Whisper turbo (Python process) | 420 / 839 MB | 547 / 967 MB |

`llama-server` b11429 defaults to an 8,192 MiB host prompt cache (`cache_ram_mib`, flag `--cache-ram`) **[code]**. An ASR server that keeps the default can hold up to 8 GiB on a machine that is also running a chat model.

### An earlier smoke test

Before this study, a smaller test ran sherpa-onnx 2.0.1 with Parakeet TDT v2 int8 and faster-whisper 1.2.1 (CTranslate2 4.8.5) `small.en` int8 on the CPU, 4 threads, on six sentences in one TTS voice. It is superseded for the engine choice, but three of its results appear nowhere else **[measured]**:

- With 1,011 hotwords (1,000 random gene like distractors plus 11 real terms), Parakeet still corrected every real term, and on pink noise it wrote a phantom sentence. faster-whisper's `hotwords` field kept only the first 223 tokens, so every real term was dropped.
- faster-whisper on the CPU took about 0.93 to 1.07 s per 3 to 5 s clip, because the encoder always processes a padded 30 s window; Parakeet's cost was about 25 ms per second of audio.
- A streaming model (Nemotron streaming English 0.6B, 560 ms chunks) used 5.4% to 7.5% of a CPU while listening and flushed in 37 to 78 ms after release, but dropped the first word without 0.5 s of leading silence, wrote numbers as words, and its partial text flickered as hotword bonuses resolved.

### What the evidence does not show

- Real speech for any engine except whisper.cpp (see [real human speech](#real-human-speech)).
- x86 CPU timings (AVX2 laptops, Windows, Linux servers); the proxy is ARM.
- A true cold load from disk; the page cache was always warm.
- BPE id prompts at 164 terms, and BPE id prompts combined with the logit bias.
- Qwen3-ASR-1.7B quantized below GitHub's 2 GiB asset limit; ggml-org publishes no smaller file.
- sherpa-onnx with a padded VAD; the VAD rows show the cost of a bad trim, not of VAD.

## Logit biasing over the whole dictionary

### Question

The prompt holds only about 45 to 50 terms, and a personal dictionary will grow past that. Can a logit booster, installed through `whisper_full_params.logits_filter_callback`, bias the decoder toward the whole dictionary? How does it compare with the prompt and with the correction pass at 14, 54, 164 and 1,000 terms, what does it do to ordinary speech and to nonspeech, and what does it cost?

### Setup

- The same whisper.cpp Metal build, `small.en` (f16) and `large-v3-turbo-q5_0`, greedy decoding with `whisper-cli`'s parameters, timestamps off, VAD off unless stated. Load average 4 to 45, so the callback's own time and an interleaved A/B comparison are the cost figures to trust.
- Gene symbols came from the HGNC complete set (`hgnc_complete_set.txt`, 16,986,126 B, SHA-256 beginning `d93ce6b2` and ending `13bed0` in the source record, 45,289 approved records, latest modification 2026-10-23).
- The booster is about 120 lines of C++ in the test program and needs no change to whisper.cpp. It runs reentrantly from concurrent decoders.

### Material and its limits

| Set | Content | Use |
|---|---|---|
| Test | the 42 clips and 2 nonspeech clips of the [synthetic dictation set](#the-synthetic-dictation-set) | results |
| Probes | 25 ordinary sentences with near homophones ("I spoke with", "PhD two years ago", "crisper drawer", "bike spokes", "Eva passed one exam") in the same 3 voices: 75 clips | harm to ordinary speech |
| Dev | 12 new sentences in two voices never used in the test set, Moira (en_IE) and Rishi (en_IN), with 60 occurrences of other dictionary terms (HIF1A, fingolimod, DESeq2, TREM2, tocilizumab and others), plus 25 probe sentences in Moira's voice | tuning only |
| Nonspeech | 6 clips: 6 s digital silence; pink noise at −40 dBFS with a 60 Hz hum; white noise at −35 dBFS; a 60 Hz hum with harmonics at −35 dBFS; two TTS sentences played backward | echo |

| Dictionary | Content | Trie nodes |
|---|---|---|
| 14 terms | the targets | 478 |
| 54 terms | plus 40 distractors (the prompt study's 54 term list) | 1,266 |
| 164 terms | plus all 150 distractors | 4,362 |
| 1,000 terms | plus 836 HGNC symbols: 8 forced near neighbours (HIF1A, HIF3A, EGLN2, EGLN3, ARNT, VEGFA, EPO, HIF1AN) and 828 sampled (seed 20261104) from approved protein coding symbols that are not English words | 20,070 |

Aliases were generated by three fixed rules for every entry (case variants, separators at letter and digit boundaries, Greek letters spelled out), never written by hand: HIF-2α got 13 and the 1,000 term dictionary 2,381. Limits: synthetic speech; a dev set of 60 occurrences; English models only.

### Method

The booster matches on **bytes, not token ids**. A trie holds each term's written form, with and without a leading space, and its aliases. For every trie node the build step precomputes the vocabulary tokens whose bytes continue some form from that node, so the match follows whatever segmentation the model emits. On each call it rebuilds the byte tail of the decoded text, finds the trie nodes reached by a suffix that starts at a word boundary, and adds gated bonuses. When the decoder leaves the trie, the bonus disappears.

**Table L1. Booster parameters, tuned on the dev set and then frozen** **[measured]**

| Parameter | Value | What it does |
|---|---|---|
| Start bonus and gate | 2.0 logits | a token that starts a written form gets +2.0 if it is within 2.0 of the top score |
| Continuation bonus and gate | 5.0 logits | a token that continues a match in progress gets +5.0 if it is within 5.0 of the top score |
| Ramp | 2 characters | the continuation bonus and gate scale with matched characters divided by 2, capped at 1 |
| First step | 0 | no start bonus at the first step of a window, which has no left context |
| Alias factor | 0 | aliases are never boosted; they serve only as exact keys in the correction pass |
| Minimum start | 2 characters | a start token must cover at least 2 characters of the term, or the whole term |
| English word start filter | on | tokens such as ` Bar`, ` Bio`, ` Nat` never start a match unless they are the whole term |
| Prefix steal guard | on | never boost a token that the model's current choice extends (` Ph` against ` PhD`) |
| Cap | 5.0 (effective) | the bonus per token is the largest single source, clamped |
| Special tokens | never boosted | end of text, timestamps and other specials are untouched |

In greedy decoding a bonus equal to its gate means "switch to a dictionary token if it is within the gate of the top token". The plateau is broad: on the dev set, start values of 1 to 3 with continuation values of 3 to 6 were within 2 hits of each other.

What tuning showed (dev set, 1,000 terms) **[measured]**:

- Without start bonuses recall stalled at 48%; with them it reached 63% to 67%. Requiring 3 characters to start cost 8 points.
- A continuation bonus of 8 dragged the decoder into wrong completions.
- Boosting aliases cost 4 to 7 points: it steers toward variant forms the correction pass must undo.
- The prefix steal guard removed the only probe damage the bias caused (`PhD` to `Ph.D.`, probe WER 5.6% back to 3.7%). The ramp removed a short prefix hijack (`Tausilizumab` to `TASIL is UMABLOAD`). No first step bonus removed `Plot` to `Clot`.
- Turbo with the frozen parameters went from 37% to 72% raw on the dev set, and dev WER from 28.9% to 13.7%.

Conditions on the test set: no dictionary; the whole dictionary as a prompt (`P_N`); the booster alone (`B_N`); a realistic top 30 prompt alone (`P30_N`); and the top 30 prompt plus the booster over all N terms (`P30B_N`). The top 30 prompt holds only 7 of the 14 targets (EPAS1, HIF-2α, Baranzini, OMOP, ocrelizumab, CRISPRi, PHD2) among 23 distractors (115 tokens on `small.en`, 118 on turbo), so the other 7 targets can be reached only through the booster. Group A is the 7 targets in the top 30 prompt; group B is the 7 reachable only through the booster. Three correction modes followed: **full** (exact and fuzzy over all N terms), **exact only** (exact keys over all N), and **hybrid** (exact over all N, then exact and fuzzy over the top 30 entries only).

### Results

**Table L2. `small.en`, strict recall and harm** **[measured]**

| Condition | Prompt tokens | Strict raw | Full correction | Group A | Group B | Test WER | Probe WER | Probe insertions, raw / full correction |
|---|---|---|---|---|---|---|---|---|
| No dictionary | 0 | 36% | 90% | 28% | 43% | 11.3% | 3.5% | 0 / 10 |
| `P_14` (prompt holds exactly the spoken terms) | 62 | 90% | 99% | 82% | 98% | 3.5% | 6.0% | 1 / 10 |
| `B_14` | 0 | 77% | 95% | 79% | 74% | 3.7% | 3.5% | 0 / 7 |
| `P_14` plus `B_14` | 62 | 99% | 100% | 97% | 100% | 0.9% | 6.3% | 4 / 12 |
| `P_164` (truncated) | 655 | 58% | 96% | 49% | 67% | 7.2% | 5.4% | 0 / 10 |
| `B_164` | 0 | 75% | 95% | 77% | 74% | 3.7% | 3.5% | 0 / 7 |
| `P_1000` (truncated) | 4,285 | 49% | 93% | 51% | 48% | 9.4% | 3.4% | 0 / 9 |
| `P30_1000` | 115 | 68% | 94% | 85% | 52% | 5.0% | 3.5% | 0 / 9 |
| **`B_1000`** | 0 | **72%** | 95% | 77% | 67% | 4.4% | **3.5%** | **0** / 10 |
| **`P30B_1000`** | 115 | **83%** | 95% | 97% | 69% | 3.1% | 4.9% | 3 / 12 |
| `B_1000`, beam 5 | 0 | 80% | 94% | 87% | 74% | 2.8% | 4.0% | 0 / 10 |

**Table L3. `large-v3-turbo-q5_0`, strict recall and harm** **[measured]**

| Condition | Prompt tokens | Strict raw | Full correction | Group A | Group B | Test WER | Probe WER | Probe insertions, raw / full correction |
|---|---|---|---|---|---|---|---|---|
| No dictionary | 0 | 43% | 91% | 31% | 55% | 13.7% | 2.0% | 0 / 9 |
| `P_14` | 63 | 84% | 99% | 95% | 74% | 2.4% | 2.6% | 0 / 9 |
| `B_14` | 0 | 77% | 93% | 87% | 67% | 3.5% | 2.0% | 0 / 6 |
| `P_14` plus `B_14` | 63 | 93% | 99% | 97% | 88% | 1.1% | 4.5% | 9 / 12 |
| `P_164` (truncated) | 677 | 54% | 93% | 44% | 64% | 9.8% | 2.2% | 0 / 9 |
| `B_164` | 0 | 74% | 93% | 82% | 67% | 3.5% | 2.0% | 0 / 6 |
| `P_1000` (truncated) | 4,559 | 52% | 93% | 49% | 55% | 10.2% | 2.0% | 0 / 9 |
| `P30_1000` | 118 | 72% | 91% | 90% | 55% | 3.7% | 2.8% | 0 / 13 |
| **`B_1000`** | 0 | **72%** | 93% | 82% | 62% | 3.5% | **2.0%** | **0** / 9 |
| **`P30B_1000`** | 118 | **77%** | 93% | 97% | 57% | 2.2% | 3.5% | 5 / 14 |
| `B_1000`, beam 5 | 0 | 70% | 93% | 79% | 62% | 3.7% | 2.2% | 0 / 8 |

- **The booster scales.** At 1,000 terms it beat the whole dictionary as a prompt by 23 points (`small.en`, 72% against 49%) and 20 points (turbo, 72% against 52%). It lost only 5 points between 14 and 1,000 terms.
- **A small prompt is still stronger for the terms it holds.** The oracle prompt `P_14` reached 90% and 84% raw, against 77% for `B_14`.
- **Prompt plus booster lifts the prompt's own terms** (group A 85% to 97% on `small.en`, 90% to 97% on turbo) but on turbo the booster's terms gained less with a prompt (57%) than without one (62%).
- **The booster alone harmed nothing measurable:** probe WER identical to no dictionary (3.5% and 2.0%), 0 probe insertions, no measurable decode time.
- **Prompt plus booster compound harm:** 3 to 9 raw probe insertions where each alone gave 0 to 1. The prompt shifts casing so an ordinary word lands on the trie, and the booster completes it: `PhD two years ago` to `PHD2 years ago`, `crisper drawer` to `CRISPRi drawer`, `The UC system` to `The UCSF announced`.
- **Beam 5 is not worth it with the booster:** `small.en` +8 points, turbo −2, with 4.7 times the callback calls and the beam's own decode time. A logits filter cannot take back a bonus once a match breaks inside a beam.

**Table L4. Correction modes at 1,000 terms** (strict recall; false insertions on target speech and probe insertions as full / exact only / hybrid) **[measured]**

| Model | Condition | No correction | Full | Exact only | Hybrid | Insertions on target speech | Probe insertions |
|---|---|---|---|---|---|---|---|
| `small.en` | none | 36% | 90% | 69% | 77% | 6 / 0 / 0 | 10 / 3 / 4 |
| `small.en` | `B_1000` | 72% | 95% | 80% | **86%** | 6 / 0 / 0 | 10 / 3 / 4 |
| `small.en` | `P30B_1000` | 83% | 95% | 86% | **88%** | 6 / 0 / 0 | 12 / 3 / 6 |
| turbo | none | 43% | 91% | 70% | 84% | 6 / 0 / 0 | 9 / 3 / 3 |
| turbo | `B_1000` | 72% | 93% | 85% | **89%** | 6 / 0 / 0 | 9 / 3 / 3 |
| turbo | `P30B_1000` | 77% | 93% | 86% | **88%** | 6 / 0 / 0 | 14 / 6 / 7 |

- **Full fuzzy correction does not scale.** At 1,000 terms its 6 false insertions on target speech were the same on every voice: `encodes` to `ENCODE`, `endothelial cells.` to `HELLS`. Its probe edits included `Boston.` to `OSTN` and `spokes` to `SPOKE`. The word list guard misses inflections because `/usr/share/dict/words` lacks `encodes` and `cells`.
- **The 3 probe insertions left under exact only are all `PhD two` to `PHD2`:** the exact key `phd2` matches the number word.
- **In a safe pipeline the booster is worth +9 and +5 points (hybrid) or +11 and +15 points (exact only)** with zero false insertions on speech. With hybrid correction the top 30 prompt adds only +2 (`small.en`) and −1 (turbo) on top of the booster.

**Table L5. Echo on six nonspeech clips, VAD off** (dictionary terms emitted) **[measured]**

| Model | No dictionary | Booster alone (14 or 1,000 terms, greedy or beam 5) | `P_14` | `P30B_1000` | `P30B_1000` with Silero VAD |
|---|---|---|---|---|---|
| `small.en` | 0 | **0** | 6 (`PHD2, PHD2, PHD2` on pink and white noise) | 2 (`PHD2A, BRCA1` from reversed speech) | 1 (reversed speech passes the VAD) |
| turbo | 0 | **0** | 3 with beam 5 (`FD2, VHL`); greedy not reported | 3 (`PTEN, APOE` from white noise) | not run |

The booster never created a term from nonspeech by itself: it boosts only tokens already within a few logits of the top, and on nonspeech the top token is a caption such as `you`. **A prompt is the echo source,** and prompt plus booster is worse than the prompt alone, because once the prompt has the model writing symbol shaped strings the booster completes them into real dictionary entries. **Silero removes silence, noise and hum but not sound that resembles speech:** reversed speech passed the gate.

**Table L6. Cost** **[measured]**

| Measure | Value |
|---|---|
| Callback time, greedy | 0.57 ms per utterance (18 to 19 calls per 4 to 6 s clip, 30 to 32 µs per call under load) |
| Callback time, beam 5 | 2.4 to 3.7 ms per utterance (about 64 calls) |
| Per call, by dictionary size (10,000 calls) | 26.5, 26.1, 26.0, 26.1 µs at 14, 54, 164, 1,000 terms; trie matching under 1 µs, the rest is the scan for the top score over 51,864 logits |
| Interleaved A/B, median ms per clip (none / booster 1,000 / top 30 prompt / both) | `small.en` 91.5 / 94.2 / 100.9 / 101.2; turbo 268.1 / 268.1 / 274.7 / 275.6 |
| Paired ratio per clip, booster against none | 1.003 (`small.en`), 1.000 (turbo) |
| Build per dictionary version | 33 to 52 ms, mostly the 50,000 token vocabulary map and the word list; memory not measured |

**Table L7. Prompt token counts: `whisper_tokenize` against canonical BPE** **[measured]**

| Model | Prompt | Terms | `whisper_tokenize` | Canonical BPE | Terms kept in the last 223 tokens | Targets kept |
|---|---|---|---|---|---|---|
| `small.en` | 14 terms | 14 | 62 | 62 | 14 | 14 |
| `small.en` | 54 terms | 54 | 209 | 211 | 54 | 14 |
| `small.en` | 164 terms | 164 | 655 | 667 | 61 / 60 | 5 |
| `small.en` | 1,000 terms | 1,000 | 4,285 | 4,364 | 51 / 50 | 1 |
| `small.en` | top 30 | 30 | 115 | 117 | 30 | 7 |
| turbo | 14 terms | 14 | 63 | 63 | 14 | 14 |
| turbo | 54 terms | 54 | 217 | 217 | 54 | 14 |
| turbo | 164 terms | 164 | 677 | 688 | 59 | 5 |
| turbo | 1,000 terms | 1,000 | 4,559 | 4,621 | 47 / 46 | 1 |
| turbo | top 30 | 30 | 118 | 118 | 30 | 7 |

- The mean cost per entry is 4.3 to 4.6 tokens. `whisper_tokenize` undercounts canonical BPE by 0% to 2%, so a budget counted with it needs about 5% headroom.
- `small.en` uses the GPT-2 English vocabulary, not `multilingual.tiktoken`: only 270 ids hold the same bytes. Each model's own vocabulary is the reference.
- **What the model emits:** with no prompt it emits canonical BPE (` EP|AS|1`); with a text prompt it copies the prompt's greedy segmentation (` EPA|S|1`) in 27 of 73 occurrences on `small.en`; some emissions match neither scheme. A trie of token ids would miss many of them, which is why the booster matches bytes.

**Table L8. Prompt as canonical ids (`prompt_tokens`) against text (`initial_prompt`)** **[measured]**

| Model | Prompt | Text, strict raw | Canonical ids, strict raw | Main changes |
|---|---|---|---|---|
| `small.en` | 14 terms | 90% | 91% | EPAS1 0 → 4 of 6, Biorouter 6 → 3 |
| `small.en` | 54 terms | 83% | 84% | EPAS1 5 → 6, Biorouter 5 → 2 |
| turbo | 14 terms | 84% | 91% | natalizumab 4 → 6, Biorouter 0 → 5 |
| turbo | 54 terms | 74% | **88%** | EPAS1 1 → 5, Biorouter 0 → 5 |

This independently confirms the engine study's tokenizer finding (E3) on turbo, with a smaller effect on `small.en` in this run.

Failure modes seen **[measured]**:

| Failure | Example | Status |
|---|---|---|
| Prefix steal | ` PhD` became ` Ph` (start of PheWAS), giving `Ph.D.` | fixed by the prefix steal guard in greedy decoding; returns under beam 5 |
| Short prefix hijack | `Tausilizumab` became `TASIL is UMABLOAD IL6` | fixed by the ramp |
| First token start | `Plot the UMAP` became `Clot the UMAP` | fixed by no first step bonus |
| Case cascade | `bio safety cabinet` became `Bio Safety Cabinet for the Tissue Culture Room` | open; 1 of 75 probes, no WER effect |
| Competition | the top token starts another dictionary term, both get the bonus, nothing changes (` SC` against ` sc`: scRNA-seq 3 of 6 at 14 terms, 0 of 6 at 1,000) | open; explains the 5 point loss from 14 to 1,000 terms. A tiered bonus is the candidate fix, not measured |
| Prompt primed completion | `PhD two years` became `PHD2 years`; `The UC system` became `The UCSF announced` | open; a per entry policy |
| Prompt primed echo | reversed speech gave `PHD2A, BRCA1` | open; VAD does not stop sound that resembles speech |
| Strong priors | turbo's `BioRouter` stays at 0 of 6 under every booster setting; lowercase ` spoke` for SPOKE | by design: the gate is what protects ordinary speech; canonical id prompts and exact key correction fix the casing cases |
| Number word collision in correction | `PhD two` to `PHD2` by exact key | open; a per entry "digits as written" flag |

Integration facts **[code]**:

- Stock `whisper-server` never sets `logits_filter_callback`, and no request field reaches one, `prompt_tokens`, a grammar or `suppress_regex`. Its only dictionary channel is the `prompt` text field.
- The booster therefore needs a decode loop Biorouter owns: a patched `whisper-server`, a helper process linked against libwhisper, or a Rust worker over whisper-rs 0.16.0 (which vendors whisper.cpp 1.8.3 through whisper-rs-sys 0.15).
- In whisper-rs 0.16.0 both callback setters, `set_filter_logits_callback` and `set_filter_logits_callback_user_data`, are `unsafe` (`src/whisper_params.rs`, lines 709 to 726 in that release). The callback's lifetime must outlive the decode, and a panic must not cross the FFI boundary.
- Token bytes must come from `WhisperContext::token_to_bytes(id)`, never from `token_to_str`, which returns an error on the partial byte tokens of byte level BPE. `token_to_cstr`, which `token_to_bytes` wraps, was public up to 0.14.x and is private in 0.16.0.
- The callback signature (`whisper_logits_filter_callback` in `include/whisper.h`) is identical between whisper.cpp 1.8.3 and 1.9.5-dev.
- The plan drives libwhisper through its own C shim, not whisper-rs ([dictation engine and models](../../dictation/engine-and-models.md)), so the whisper-rs facts above matter only if that choice is revisited.
- A helper must ship its own English word list with inflections: `/usr/share/dict/words` is missing on Windows and in slim Linux images, and it lacks `encodes` and `cells`.

### What the evidence does not show

- Any of this on real speech. The gate is relative to the top score, so on noisier speech with flatter score distributions both the gain and the false change rate may rise **[inferred]**.
- The booster combined with a canonical id prompt; only the text prompt combination was run.
- The booster with entries that collide with English words excluded (the proposed default for SPOKE, PHD2, CRISPRi, UCSF and Biorouter).
- Multilingual models with automatic language detection; long recordings with timestamps on (coded, not measured); concurrent decoding through `whisper_full_parallel`.
- The tiered bonus that might recover the 5 point loss at 1,000 terms.

## Real human speech

### Question

Do the synthetic findings hold on recorded human speech: does the prompt add 40 to 50 points of strict recall, does VAD remove echo, and does the correction pass make no false edits? Which public corpora can Biorouter use for evaluation and regression tests, and how should the lab record its own set?

### Setup

- The same whisper.cpp Metal build; `small.en` (f16) and `large-v3-turbo-q5_0`; Silero v6.2.0.
- A copy of the prompt study's test program with two additions: a prompt per clip, and a switch that turns the GPU off. Everything else unchanged: greedy with best of 5, temperature fallback +0.2, entropy 2.4, log probability −1.0, no speech threshold 0.6, timestamps off unless stated, English, 8 threads.
- Load average 4 to 116 during the runs, so latency is order of magnitude only.

### Material and its limits

**Table R1. Public corpora surveyed** (checked 2026-11-04)

| Corpus | License | Commit a few clips to the public repo | Biomedical term density | Verdict |
|---|---|---|---|---|
| [PriMock57](https://github.com/babylonhealth/primock57) at `cd2ac707` | CC BY 4.0 | yes, with attribution | drug names only (about 226 mentions in 8 h 38 min of consultations); 0 gene symbols | the only real, medical, per utterance aligned set that may be committed |
| [Omi Health medical-STT-eval](https://github.com/Omi-Health/medical-STT-eval) | code MIT; the data is PriMock57 | as PriMock57 | as PriMock57 | a scorer and a curated term list, not new audio |
| Fareez et al. 2022, simulated clinical interviews | CC0 on the figshare article (the collection record has no license field) | yes | mostly respiratory; inferred to hold inhaler and antibiotic names | not downloaded in this study. The `figshare.com/ndownloader/...` web path answers a scripted client with an AWS WAF challenge (HTTP 202), but the figshare API's own download URL (`ndownloader.figshare.com/files/30598530`) served the 1,034,153,474 B `Data.zip` to a scripted client in a later check. Transcripts have no timestamps, so clips need forced alignment |
| Kaggle "Medical Speech, Transcription, and Intent" | no license stated | no | symptoms only | not usable |
| [MultiMed](https://huggingface.co/datasets/leduckhai/MultiMed) English | card says MIT, but the audio is from YouTube channels that did not license it | no | **the only public real speech with spoken gene symbols found:** 621 HGNC symbol tokens in about 32,000 utterances | internal measurement only |
| Common Voice English 23.0 | CC0 plus terms of service | avoid | counted on the English sentence corpus the prompts are drawn from (`wiki.en.txt` and `sentence-collector.txt`, about 1.36 million lines), not on the 23.0 recordings: 0 sentences with digits, 0 gene symbols, rare drug names. Which of these sentences were recorded is unknown without the 87 GB download | not useful |
| People's Speech, LibriSpeech | CC BY (subsets) | yes | counted on People's Speech's test split (33,398 rows read) and LibriSpeech test-clean plus test-other (5,359 rows), not the whole corpora: almost no drug names, 0 gene symbols; People's Speech is lowercase | not useful |
| M3AV (NIH lectures) | annotations CC BY-NC-SA; video rights stay with their owners | no | inferred to be gene dense | not measured |
| AfriSpeech-200 | CC BY-NC-SA, gated | no | inferred to be drug dense | not measured |

Nothing public has dense gene symbol dictation. Some real transcripts contain words that trip Biorouter's retired name guard (4 lines in MultiMed, 287 lines of the Common Voice sentence corpus), so any committed transcript must be screened.

**Material downloaded**, each into its own empty directory, only the bytes needed:

| Set | Content | Limits |
|---|---|---|
| PriMock57 speech | 60 utterances (44 doctor, 16 patient), 420.6 s, mean 7.0 s, 85 drug occurrences over 26 terms. Selected for a drug name, no "unsure" or "unintelligible" tags, 2 to 25 s. Per clip SHA-256 recorded. | British and European accents, laptop and phone microphones, audio passed through Opus at a variable bitrate before WAV export, drug names only |
| PriMock57 nonspeech | 16 clips, 80 s: 10 spans where both channels are silent (real room tone) and 6 where only the other party speaks | room tone passed through Opus |
| MultiMed English | 50 utterances with gene symbols (25 test, 25 eval, at most 3 rows per gene), 608.1 s, mean 12.2 s, 78 occurrences (77 without one row whose audio does not match its text); 37 terms. Converted from 44.1 or 48 kHz stereo to 16 kHz mono. | lecture and talk audio, not dictation; reference errors and mixed casing; internal use only |

Prompts were comma lists of the terms present in each set, most frequent last: 26 drug terms (111 tokens on `small.en`, 119 on turbo) and 37 gene terms (149 and 160 tokens). Both fit the budget. Comparisons use an exact sign test per occurrence (two sided).

### Results

**Table R2. PriMock57: drug names** (85 occurrences) **[measured]**

| Model | Condition | Strict | Lenient | False insertions | WER | Nonspeech with text / with dictionary text |
|---|---|---|---|---|---|---|
| `small.en` | no prompt | 49% | 51% | 0 | 15.3% | 2 / 0 of 16 |
| `small.en` | glossary | 74% | 76% | 0 | 13.3% | 4 / 0 |
| `small.en` | glossary, beam 5 | 74% | 76% | 1 | 13.5% | 4 / 0 |
| `small.en` | VAD | 47% | 47% | 0 | 15.0% | 0 / 0 |
| `small.en` | **VAD and glossary** | **79%** | 80% | 0 | **11.1%** | 0 / 0 |
| `small.en` | timestamps on, VAD and glossary | 78% | n/a | n/a | 16.2% | 0 |
| turbo q5_0 | no prompt | 74% | 75% | 0 | 10.2% | **16** / 0 |
| turbo q5_0 | glossary | 80% | 82% | 0 | 9.5% | 16 / 0 |
| turbo q5_0 | glossary, beam 5 | 80% | 82% | 0 | 10.1% | 16 / 0 |
| turbo q5_0 | VAD | 76% | 79% | 0 | 9.4% | 1 / 0 |
| turbo q5_0 | **VAD and glossary** | **82%** | 84% | 0 | 10.3% | 1 / 0 |
| turbo q5_0 | timestamps on, VAD and glossary | 81% | n/a | n/a | 9.6% | 1 |

**Table R3. MultiMed English: gene symbols** (77 occurrences) **[measured]**

| Model | Condition | Strict | WER | Collapsed utterances (of 49) | Nonspeech with text (of 16) |
|---|---|---|---|---|---|
| `small.en` | no prompt | 42% | 23.1% | 1 | 2 |
| `small.en` | glossary | 70% | **37.7%** | **12** | 7, gene symbol strings in 5 |
| `small.en` | glossary, beam 5 | 82% | 27.1% | 5 | 8 |
| `small.en` | VAD | 45% | 21.3% | 1 | 0 |
| `small.en` | VAD and glossary | 70% | 33.6% | 10 | 0 |
| `small.en` | timestamps on, no prompt | 52% | 20.3% | 0 | 0 |
| `small.en` | timestamps on, glossary | 75% | 26.7% | 2 | 2 |
| `small.en` | timestamps on, VAD and glossary | 74% | 26.7% | 2 | 0 |
| `small.en` | `Glossary: … .` frame | 75% | 31.9% | 1 | 13 (`Thank you.`) |
| `small.en` | 12 term prompt | 51% | 47.9% | **17** | 3 |
| `small.en` | only the clip's own terms | 62% | 35.9% | 7 | 2 |
| turbo q5_0 | no prompt | 58% | 17.1% | 0 | 16 |
| turbo q5_0 | **glossary** | **92%** | **15.0%** | 0 | 15 |
| turbo q5_0 | glossary, beam 5 | 94% | 15.0% | 0 | 15 |
| turbo q5_0 | VAD | 58% | 17.6% | 0 | 1 |
| turbo q5_0 | VAD and glossary | 91% | 15.1% | 0 | 1 |
| turbo q5_0 | timestamps on, VAD and glossary | 92% | 15.4% | 0 | 1 |
| turbo q5_0 | `Glossary: … .` frame | 86% | 18.0% | 1 | 16 |
| turbo q5_0 | only the clip's own terms | 74% | 28.4% | 1 | 16 |

**Table R4. Paired tests** (per occurrence, strict) **[measured]**

| Set | Model | Comparison | Strict | Gained / lost | p |
|---|---|---|---|---|---|
| PriMock57, all drugs | `small.en` | no prompt → glossary | 49% → 74% | 23 / 2 | 1.9e-5 |
| PriMock57, all drugs | `small.en` | VAD → VAD and glossary | 47% → 79% | 28 / 1 | 1.1e-7 |
| PriMock57, rare drugs (46 occurrences without paracetamol, ibuprofen, aspirin) | `small.en` | no prompt → glossary | 26% → 65% | 19 / 1 | 4.0e-5 |
| PriMock57, rare drugs | `small.en` | VAD → VAD and glossary | 22% → 72% | 23 / 0 | 2.4e-7 |
| PriMock57, all drugs | turbo | no prompt → glossary | 74% → 80% | 8 / 3 | 0.23 |
| PriMock57, rare drugs | turbo | no prompt → glossary | 57% → 70% | 8 / 2 | 0.11 |
| PriMock57 | either | glossary → glossary with beam 5 | no net change | 2 / 2 and 1 / 1 | 1.0 |
| MultiMed | `small.en` | no prompt → glossary | 42% → 70% | 24 / 2 | 1.0e-5 |
| MultiMed | turbo | no prompt → glossary | 58% → 92% | 26 / 0 | 3.0e-8 |
| MultiMed | turbo | VAD → VAD and glossary | 58% → 91% | 26 / 1 | 4.2e-7 |

What R2 to R4 show:

- **The prompt holds on real speech, at a size that depends on the model and the terms.** Gene symbols gained 24 to 33 points. Drug names gained 25 to 32 points on `small.en` and 39 to 50 points on the rare drugs alone, but only 6 points on turbo, because 46% of the drug occurrences are paracetamol, ibuprofen or aspirin, which Whisper already knows.
- **The prompt fixes real hearing errors, not only format.** Without a prompt turbo wrote `FLAT3` (FLT3), `ALAX5AP` (ALOX5AP), `PREM` (PRAME), `cruzartinib` (quizartinib); with the glossary it wrote them correctly. Lenient recall without a prompt was only 49% to 65% on genes, so a format fixer alone cannot close the gap. It also fixed brand names (Ventolin 0 of 5 to 4 or 5 of 5 on `small.en`). It did not fix everything: Clenil stayed at 0 to 1 of 5 on turbo.
- **Beam 5 added nothing on real speech** (the synthetic set showed 0 to 7 points).
- **Turbo beats `small.en` on real speech,** reversing the synthetic result: 91% to 92% against 70% to 75% on gene symbols, 80% to 82% against 74% to 79% on drugs, and turbo never collapsed.
- **Collapse is a new failure on `small.en`.** With the gene glossary and timestamps off, `small.en` returned just `and` or `,` for 12 of 49 utterances; a 46 word reference came back as `and`. Prompt length does not explain it (a 12 term prompt collapsed 17). Timestamps on cut collapses from 12 to 2, and a `Glossary: … .` frame to 1. The drug prompt on `small.en` collapsed 0, so the trigger is the content: a list of uppercase letter and digit symbols, which is exactly a biomedical dictionary.
- **Prompt continuation deletion is a new failure on both models.** Whisper treats the prompt as the preceding transcript, so when the speech begins with the terms the prompt ends with, it skips them: with the prompt `HK2`, "…the relationship between the HK2 and AKT" came out as "and AKT." A prompt narrowed to the clip's own terms cost turbo 18 points and doubled its WER. A natural language tail ("New dictation follows.") leaked into the output.
- **A prompt term can intrude into a mishearing:** `paracetamol` heard as `piracy tomorrow` became `piracy to Moduli` with the drug glossary, a fragment of the prompt term Modulite.

**Table R5. Real nonspeech** (16 PriMock57 gaps, the same clips in every condition) **[measured]**

| Model | Without VAD | With VAD |
|---|---|---|
| turbo q5_0 | text on 15 to 16 of 16 clips in every prompt condition: `Thank you.` most often; with the drug glossary also `Grazie` (11 times), `Grazie a tutti`, `Sous-titrage Société Radio-Canada`, `Gracias por ver el video` and `Altyazı M.K.`, although the language was forced to English | 1 of 16 in every condition, probably an untranscribed backchannel **[inferred]** |
| `small.en` | tags (`[BLANK_AUDIO]`, `[ Silence ]`, `(air whooshing)`) and `Okay.`; with the gene prompt, symbol strings on 5 to 6 of 16 clips: `PPP4, PPP4, CXCR4,` and `CXCR4, CXCR4, CXCR4, CXCR4,` | 0 of 16 in every condition |

- PPP4 is a real HGNC symbol that was not in the dictionary, so the prompt induces symbol shaped inventions, not only copies.
- VAD did not hurt speech recall: 49 to 47 and 74 to 76 on PriMock57, 42 to 45 and 58 to 58 on MultiMed, all within noise (p 0.25 or more).

**Table R6. Correction pass on the real transcripts** (guard on) **[measured]**

| Set | Model | Input | Strict before → after | Edits | False edits |
|---|---|---|---|---|---|
| PriMock57 | `small.en` | no prompt | 49% → 71% | 20 | 0 |
| PriMock57 | `small.en` | glossary | 74% → 86% | 13 | 1 (`Moduli` → Modulite, a prompt artifact) |
| PriMock57 | turbo | no prompt | 74% → 89% | 16 | 0 |
| PriMock57 | turbo | glossary | 80% → 96% | 15 | 0 |
| MultiMed | `small.en` | no prompt | 41% → 72% | 26 | 1 (`N-TRAC` → **NRAS**; the speaker said NTRK) |
| MultiMed | turbo | no prompt | 58% → 85% | 23 | 1 (same) |
| MultiMed | turbo | glossary | 91% → 95% | 4 | 0 |
| MultiMed | turbo | VAD and glossary | 90% → 91% | 2 | 0 |

**Table R7. Correction pass over whole real text corpora** (every edit that writes a term the reference lacks is false) **[measured]**

| Dictionary | Corpus | Guard | False edits | Examples |
|---|---|---|---|---|
| 26 drug terms | 7,108 PriMock57 utterances | on | **0** (7 edits fixed typos in the reference) | |
| 26 drug terms | same | off | 28 | `clinic` → Clenil ×15, `continue` → codeine ×9 |
| 37 gene terms | 4,651 MultiMed test sentences | on | **45** | `here's` → HRAS ×21, `aren't` and `ain't` → ARNT ×12, `ras` → HRAS ×4, `norms` → NRAS ×2 |
| 37 gene terms | same | off | 100 | adds `prime` → PRAME ×7, `brief` → BRAF ×7, `motor` → MTOR ×6 |
| the 14 synthetic set terms | 7,108 PriMock57 utterances | on / off | 0 / 52 | off: `smoke` → SPOKE ×48 |
| the 14 synthetic set terms | 4,651 MultiMed test sentences | on | 13 | `UCSD` → UCSF ×4, `exon 1` → EGLN1 ×3, `spikes` → SPOKE ×2, `using RNA-seq` → scRNA-seq ×2, `CRISPR` → CRISPRi |

The guarded fuzzy pass is safe for long coined drug names and **unsafe for short gene symbols** (HRAS, NRAS, ARNT, BRAF, PRAME, MTOR) and for acronyms one edit away from another institution or gene (UCSF and UCSD, NRAS and NTRK, EGLN1 and "exon 1"). `/usr/share/dict/words` (a 1934 word list) lacks "coding", "here's" and "aren't", so the guard let them through. Exact key normalization (casing, hyphens, spacing, `IGF-1R` to IGF1R, `P-R-O-K-R2` to PROKR2) made no false edit in any run.

**Table R8. Latency on real clips, CPU only against Metal** (12 PriMock57 clips, mean 8.1 s, glossary prompt, M4 Max under load) **[measured]**

| Model | Metal, 8 threads | CPU only, 8 threads | CPU only, 4 threads |
|---|---|---|---|
| `base.en` | 261 ms (RTF 0.032) | 337 ms (0.042) | 438 ms (0.054) |
| `small.en` | 485 ms (0.060) | 636 ms (0.078) | 1,452 ms (0.179) |
| `large-v3-turbo-q5_0` | 439 ms (0.054) | **2,869 ms (0.354)** | 2,739 ms (0.338) |

These CPU runs used Apple's Accelerate BLAS, so Intel Mac, Windows and Linux CPUs will be slower **[inferred]**. They agree with the engine study's portable CPU build: turbo is several seconds per utterance on a CPU; `small.en` and `base.en` stay near or under 1 s.

**Table R9. whisper.cpp's own test assets, read in its source tree** (candidates for offline CI fixtures) **[measured]**

| File | Bytes | SHA-256 | What it is |
|---|---|---|---|
| `models/for-tests-ggml-base.en.bin` | 586,836 | `1bc042ca584ff1895897e95bffb34ccf357be46c1fca97cf7fbe32f2060aa9e8` | header, mel filters and the full English vocabulary, no weights |
| `models/for-tests-ggml-tiny.bin` | 575,451 | `c486fb9f14a28b1c1dc252741a431646cc573450c900b9d9c406e10294aa01e6` | the same with the multilingual vocabulary |
| `models/for-tests-silero-v6.2.0-ggml.bin` | 885,098 | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` | identical to the real Silero v6.2.0 model |
| `samples/jfk.wav` | 352,078 | `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` | 11.0 s of 16 kHz mono speech |

- Loading the empty base model and decoding `jfk.wav` returned success, 0 segments and empty text in about 130 ms.
- **`whisper_tokenize` with the empty models gives exactly the production token counts** (111, 149, 62 with the English vocabulary; 119, 160, 63 with the multilingual one), so prompt budget logic can be tested offline with a 0.6 MB file per tokenizer family.
- The vocabulary inside these binary files holds two tokens that Biorouter's retired name guard looks for. `git grep -I`, which the guard uses, skips the files because a NUL byte appears within the first 8 bytes. A tokenizer shipped as text would trip the guard **[measured]**.

From this material the study wrote a protocol, since withdrawn, for the lab's own recording set (3 to 5 speakers, about 60 read sentences, 15 semi spontaneous prompts, 3 long recordings and 5 nonspeech clips each, tiered consent, no PHI, stored as private BaranziniLab release assets) and a three tier CI strategy (offline text and plumbing tests on every pull request, a nightly networked accuracy job on the PriMock57 subset, a release run on the lab set). The plan for the CI tiers is in [dictation testing and release](../../dictation/testing-and-release.md). The owner withdrew the lab recording set on 2026-11-05 (O-23): Biorouter records no real voices for evaluation, so the release run uses the public recordings above and synthetic speech instead.

### What the evidence does not show

- Dictation into a composer: the sets are consultations and lectures.
- Gene symbols in dictation density, American, East Asian and South Asian accents, or Biorouter's own capture path (browser or Electron audio processing, Bluetooth headsets).
- The canonical id prompt, the logit booster, Qwen3-ASR or sherpa-onnx on real speech.
- Turbo q8_0 or `small.en-q5_1` on real speech; only `small.en` f16 and turbo q5_0 ran.

## What the numbers support

Each conclusion names the tables that support it. Where the numbers do not settle a point, it says so. The decisions themselves are made in the [implementation plan](../../dictation/implementation-plan.md)'s decision register.

### Engine

- **whisper.cpp, through libwhisper, in a decode loop Biorouter owns.** Two of the measured gains need access that stock `whisper-server` does not give: the prompt as canonical BPE ids through `prompt_tokens` (+13 points pooled and +5 on gene symbols for turbo, E3; +7 to +14 on turbo, L8) and the logit booster through `logits_filter_callback` (L2, L3). The server's only dictionary channel is a text prompt that goes through the greedy tokenizer **[code]**. Whether the loop runs in a worker process is decided by crash isolation and packaging, not by these numbers; the logit study's integration notes assume a worker **[inferred]**.
- **whisper.cpp is the only engine measured on real speech** (R2 to R4), and with the canonical id prompt it has the highest pooled raw recall of every arm that ran on all four synthetic conditions (E1).
- **Qwen3-ASR-0.6B is rejected.** It wrote one listed gene in place of another even at 14 terms, dumped the dictionary into speech at 54 terms or more, was the least noise tolerant model without a dictionary, and is larger than turbo q5_0 (E1, E4, E5).
- **Qwen3-ASR-1.7B is at most an optional tier for very large prompts on the desktop app.** It is the only measured method that held recall at 54 terms without the canonical id fix and degraded gently at 164 (E4, E5), but it needs a second `llama-server` with `--cache-ram 0` (E8), 2.5 GB of downloads with one file over GitHub's 2 GiB asset limit, and it substituted a listed term once (E1). The booster now covers large dictionaries inside whisper.cpp on synthetic speech (L2, L3).
- **sherpa-onnx with Parakeet hotwords is the fastest CPU engine with a dictionary** (about 0.12 s per utterance, 88% pooled at score 2.5; E1, E7) but would be a second native engine with onnxruntime (about 33 MB of libraries), a CC-BY-4.0 model and its own failure modes (hotword capture at 164 terms, alias loops; E4, E6). The numbers do not justify it for a first release.
- **whisper.cpp's Parakeet port is rejected** for dictation with a dictionary: it has no biasing interface, and the correction pass alone stops at 87% (E1).
- **transcribe.cpp offers no advantage** once the tokenizer cause is fixed in whisper.cpp (E3).

### Model tiers per hardware class

| Hardware | Tier | Model | Download | Evidence |
|---|---|---|---|---|
| Apple Silicon (Metal) | Default | `large-v3-turbo-q5_0` | 574,041,195 B | best on real speech (R2, R3; never collapsed); best gene symbols (E2, E5); 275 ms warm, 0.94 GB RSS (E7, E8); multilingual |
| Apple Silicon (Metal) | Light | `small.en-q8_0` | 264,477,561 B | equal to f16 and q5_1 on synthetic speech (E2); 127 ms, 0.7 GB (E7, E8); weaker in noise (E2) and on real gene speech (R3); English only |
| CPU only: Intel Mac, Windows, Linux desktop, servers | Default | `small.en-q8_0` | 264,477,561 B | 0.726 to 0.745 s per utterance on the 4 thread portable build (E7); q5_1 matched it in accuracy (E2) but needs 2.8 times its encoder work on a CPU (see the note below). Turbo q5_0 is 9 to 9.6 s there and 2.7 to 2.9 s per 8 s clip even with Accelerate (E7, R8) |
| Any | Measured; not in the v1 catalog (see the plan's DD-14) | `base.en` | 147,964,211 B | weaker prompt following (72% against 90% for `small.en`, P1); a candidate only where memory or CPU is very short |
| Any | Not shipped | `small.en` f16 and q5_1, turbo q8_0, `medium.en-q5_0` | 190 MB to 874 MB | no measurable accuracy gain over the shipped files (E2); `small.en-q5_1` is slower than q8_0 on a CPU; `medium.en` gained nothing over turbo (P1) and lost sentences in long recordings (P5) |

- **Quantized files cost no accuracy** (E2): on Metal, where speed is the same, turbo q5_0 is the smaller download. Every file in the table is under GitHub's 2 GiB asset limit and is pinned here by size and SHA-256, which an in house release can reuse.
- **On a CPU the quantization type sets the speed.** A later CPU study on the same portable build, outside the four recorded here, measured the q5_1 encoder at 2.8 times the single core work of q8_0 (6.95 s against 2.46 s per 30 s window for `small.en`; turbo q5_0 34.7 s against q8_0 12.3 s). So the 0.73 s figure belongs to q8_0, and q5_1 is likely slower on a CPU **[inferred]**. On that evidence the plan pins q8_0 for CPU hosts and the Light tier; the owner confirmed q8_0 as the CPU default on 2026-11-05, and the plan's measurement M-10 measures its latency on x86.
- **Decode with timestamps on for every model.** It cost turbo nothing on real speech (R2, R3) and cut `small.en`'s collapses from 12 to 2 (R3). The engine study saw no difference on short synthetic clips (E3), and the prompt study saw sentence loss in long recordings with timestamps off (P5).
- **Keep every request at 25 s or less,** split by VAD or by push to talk (P5).
- **Not settled by these numbers:** the CPU default's real latency on x86, and whether turbo q5_0 or q8_0 is better on real speech. Both are in [measurements still missing](#measurements-still-missing).

### Biasing mechanism and its parameters

- **The dictionary must reach the decoder.** A correction pass alone stopped at 87% on an engine with no biasing (E1), and on real gene speech lenient recall without a prompt was only 49% to 65% (R3), so most misses were hearing errors, not format.
- **The prompt, as decided:**
  - a comma list of written forms, with no natural language tail (R3);
  - passed as canonical BPE token ids from the model's own vocabulary, not as text (E3, L8);
  - at most about 30 entries and about 120 to 150 tokens, counted with 5% headroom over `whisper_tokenize` (L7), because accuracy falls before the 223 token wall (P1, E4);
  - most important terms last, because the front is what gets dropped (P1);
  - not narrowed to the terms predicted for this utterance: keep pinned and recent entries plus enough other entries that the last items are unlikely to be the first words spoken (R3, prompt continuation deletion);
  - a long transcript that is far shorter than its speech implies is decoded once more without the prompt, with VAD still on (R3).
- **The logit booster, as measured:** the parameters in L1, greedy decoding, over the whole dictionary after the privacy filter. On synthetic speech it carried 1,000 terms (72% against 49% to 52% for the same dictionary as a prompt), harmed no probe sentence and echoed nothing on its own, at about 0.6 ms per utterance (L2, L3, L5, L6). **Not decided by the numbers:** its effect on real speech, and how it combines with the canonical id prompt. With a text prompt the combination raised probe insertions from 0 to 3 to 9 and made echo worse (L2, L3, L5); excluding entries that collide with English words from the booster is the proposed fix and is unmeasured.
- **Greedy decoding.** Beam 5 added 0 to 7 points on synthetic speech (P1), nothing on real speech (R4), and with the booster +8 or −2 at several times the cost (L2, L3).
- **The bias set follows the privacy tier.** Dictionary text reached transcripts on nonspeech without a gate on every engine (P3, E6), on speech as substituted or completed terms (E4, L2, L3), and as prompt induced inventions on real room tone (R3, R5). So the prompt, the booster and the correction keys for a chat must leave out entries of private origin when the chat's capability is public. Privacy tiers are a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary. See [privacy tiers](../../security/privacy-tiers.md).

### Voice activity detection

- **Silero v6.2.0 (`ggml-silero-v6.2.0.bin`, 885,098 B, MIT) runs on every request, as a gate.** It made every synthetic nonspeech decode empty (24 of 24 in P3, 40 of 40 in E6), cut turbo's output on 16 real gaps from 15 or 16 of 16 down to 1 of 16 and `small.en`'s from 2 to 8 of 16 (5 to 6 of them gene symbol strings under the gene prompt) to 0 (R2, R3, R5; the gaps are 10 room tone and 6 bleed clips, and bracketed tags are not counted), and cost no speech recall (the notes under P1, E6, and the notes under R5). It returns nonspeech in about 10 to 15 ms instead of 0.7 to 3.5 s of fallback loops (P6, E7).
- **Pad the speech, never trim tightly.** whisper.cpp's integrated VAD with its 30 ms padding cost nothing; an unpadded trim cut first words (E6). An external trimmer should keep about 0.3 s **[inferred from E6]**.
- **No retry without VAD.** Every engine wrote dictionary text from silence or noise when ungated (E6).
- **VAD is not enough on its own.** Reversed speech passed the gate and still produced dictionary symbols with a prompt (L5). A second filter must drop outputs that are only bracketed tags or known caption hallucinations, including the ones in other languages seen on real room tone (`Grazie`, `Sous-titrage Société Radio-Canada`, `Gracias por ver el video`; R5).

### Correction pass

- **Exact key normalization over the whole dictionary, always.** It made no false edit on speech in any run, synthetic or real (P4, L4, R7), with one exception on ordinary text: a number word (`PhD two` to `PHD2`; P4, L4). Entries whose written form ends in a digit need a "digits as written" rule.
- **Fuzzy and phonetic matching only per entry, and only over the active top 30 set.** Full fuzzy correction over 164 or 1,000 terms inserted false terms into correct text (E4, L4), and on real text the guarded pass made 45 false edits with 37 gene terms (R7). Hybrid correction kept recall with zero false insertions on speech (L4).
- **Never fuzzy match an entry of 5 characters or less or with a letter and digit shape** (HRAS, NRAS, ARNT, BRAF, MTOR, EGLN1). That rule would have stopped every gene false edit measured (R6, R7).
- **Refuse fuzzy edits whose source is a modern English word, an inflection or a contraction,** using a modern word list shipped with the engine (L4, R7), and keep a stop list of measured near collisions: UCSD and UCSF, NTRK and NRAS, "exon 1" and EGLN1, CRISPR and CRISPRi (P4, R7).
- **Spoken aliases are correction keys only, with their own case and context policy.** Fed to a decoder they lowered recall and caused loops (E1, E6, and the tuning notes after L1); matched without case or context they rewrote `PhD two`, `Oh mop it up` and `the bio router in the closet` (the alias notes after E6).
- **Never chain a fuzzy edit onto a prompt fragment** (`Moduli`, R6), and show every correction as an undoable edit, because a wrong correction is worse than a visible mishearing **[inferred]**.

## Measurements still missing

1. **The lab recording set: withdrawn.** The owner decided on 2026-11-05 that Biorouter records no real voices for evaluation (O-23), so the lab recording set (50 to 100 dictated sentences from 3 to 5 Baranzini Lab members) will not be made: no `BaranziniLab/biorouter-dictation-evalset`, no IRB step and no consent form. Biorouter never publishes an accuracy figure for dictation in user documentation, release notes, announcements or the interface. Design measurements in `docs/dictation/` and `docs/research/`, this document among them, stay, labeled measured and dated. The [inferred] and synthetic only results above are checked instead on public recordings and synthetic speech (M-22 and Tier 1 in [dictation testing and release](../../dictation/testing-and-release.md)); what those cannot reach stays unsettled: gene symbols at dictation density, accents beyond the public sets, and Biorouter's own capture path.
2. **The canonical id prompt and the logit booster on real speech,** each alone and together, with entries that collide with English words excluded from the booster. The canonical id prompt plus the booster was never run on synthetic speech either. The plan's measurement M-22 runs them on the PriMock57 subset and the MultiMed English gene subset.
3. **x86 CPU latency** for `small.en-q8_0`, turbo q8_0 with the encoder window sized to the utterance, and `base.en` on an AVX2 laptop, a Windows machine and a Linux server, and Metal latency on base M1 or M2 Macs. All latency here is from one M4 Max.
4. **Turbo q5_0 against q8_0, and `small.en-q5_1` against f16, on real speech.**
5. **A true cold disk load:** model load from disk with the page cache purged. The Metal shader compile is no longer missing: the prompt study saw about 2.5 s once (P6), and a later measurement put the first worker at 3.0 to 4.2 s to ready (4,168 ms after the cache entry was evicted), of which 2.8 to 3.9 s was the compile. It recurs after a model install, an app update that changes ggml's kernel sources, or eviction from the shared Metal cache, not after a reboot; the cost measured after a reboot was about 1.3 s for the first run of `biorouterd` (see [dictation engine and models](../../dictation/engine-and-models.md)).
6. **Browser and Electron audio processing** (echo cancellation, noise suppression, automatic gain) on against off, and Bluetooth headsets in 16 kHz call mode. This needs speech through Biorouter's own capture path, which only a recording of a person would give and the owner's decision O-23 (2026-11-05) does not allow, so it stays unsettled and the plan's defaults stand unmeasured.
7. **Long recordings through the real pipeline:** VAD segmentation to 25 s or less with timestamps on, and the thresholds of the short transcript detector for prompt continuation deletion.
8. **Correction thresholds on held out text,** and the effect of a modern word list with inflections on the guard.
9. **Multilingual decoding** with turbo and automatic language detection, with and without the booster.
10. **Booster refinements:** the tiered bonus for the competition failure, memory of the trie at 1,000 terms, and the `whisper_full_parallel` path.
11. **Live partial text** for whisper.cpp; only a streaming Nemotron smoke test exists.
12. **Engines not chosen,** if they return to consideration: sherpa-onnx with a padded VAD, Qwen3-ASR-1.7B quantized under 2 GiB, and selection that keeps near twin symbols out of one Qwen3-ASR request.
13. **More public real speech:** the CC0 Fareez interviews (a scripted download through the figshare API's download URL, plus forced alignment) and the gene dense NIH lectures in M3AV, if the owner accepts downloading them for internal tests.

## Related documentation

- [Voice dictation research](README.md): the index of this folder and what each research document covers.
- [Dictation tools survey](dictation-tools-survey.md): how other engines and dictation apps work, the context these measurements test.
- [Voice dictation](../../dictation/README.md): Biorouter's dictation design, which applies these results.
- [Dictation engine and models](../../dictation/engine-and-models.md): the engine placement and model catalog chosen from the tiers above.
- [Dictation dictionary](../../dictation/dictionary.md): the dictionary schema that carries the match, case and alias policies these measurements require.
- [Dictation testing and release](../../dictation/testing-and-release.md): the CI tiers and accuracy gates built from the real speech study, on public recordings and synthetic speech.
- [Llama Server model catalog](../../providers/llama-server/model-catalog.md): how Biorouter already pins local models by size and SHA-256, and the b11429 build the Qwen3-ASR runs used.
- [Privacy tiers](../../security/privacy-tiers.md): the tier rule that keeps private entries out of a public chat's bias set.
