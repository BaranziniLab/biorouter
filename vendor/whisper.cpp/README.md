# Vendored whisper.cpp

The speech recognition engine behind Biorouter's local dictation. `source/`
holds upstream [whisper.cpp](https://github.com/ggml-org/whisper.cpp) at the
commit `pin.json` names, unmodified, limited to the files the build reads plus
four of upstream's test files. A build reads it with no network, so an upstream
repository that is deleted, force pushed or changed cannot affect Biorouter,
and the exact bytes compiled into `biorouter` and `biorouterd` are reviewable in
this repository's history. It is laid out the way `vendor/computer-use/` is: a
pristine tree, a patch folder, a pin and a hash manifest.

| Path | What it is |
|---|---|
| `source/` | upstream's files at the pin, never edited by hand |
| `patches/` | Biorouter's changes to upstream, applied to a copy at build time; empty in v1 |
| `pin.json` | `upstream`, `commit`, `contains`, `ggml` and `date` |
| `source-manifest.json` | the path, size and SHA-256 of every file under `source/`, one digest over the whole tree, and every upstream folder left out |
| `LICENSE` | upstream's MIT text, byte identical to `source/LICENSE` |
| `NOTICE` | what is vendored, the files that carry their own notices, and the licence Biorouter takes miniaudio under |

## What is in `source/`

The top level `CMakeLists.txt`, `cmake/`, `include/`, `src/` and `LICENSE`;
`ggml/` with its `CMakeLists.txt`, `cmake/`, `include/` and `src/`; and
`examples/miniaudio.h`, which only the terminal's capture crate compiles. ggml
enters a backend folder only when that backend's `GGML_<BACKEND>` option is on,
and the build turns on only CPU (everywhere) and Metal (macOS arm64), so every
other folder under `ggml/src/ggml-*/` is left out: 177 files and 7,249,911 B
under `ggml/`, against 1,484 files and 23,517,546 B for upstream's whole folder.
The build sources come to 216 files and 12,065,668 B.

The four fixtures are upstream's own test assets at the pin, 2,399,463 B in all:

| File | Bytes | SHA-256 |
|---|---:|---|
| `models/for-tests-ggml-base.en.bin` | 586,836 | `1bc042ca584ff1895897e95bffb34ccf357be46c1fca97cf7fbe32f2060aa9e8` |
| `models/for-tests-ggml-tiny.bin` | 575,451 | `c486fb9f14a28b1c1dc252741a431646cc573450c900b9d9c406e10294aa01e6` |
| `models/for-tests-silero-v6.2.0-ggml.bin` | 885,098 | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` |
| `samples/jfk.wav` | 352,078 | `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` |

The two `for-tests` models carry a real vocabulary and no weights, so the
loader, the tokenizer and `whisper_full` can be exercised offline. The detector
has real weights and is byte identical to the production detector Biorouter
downloads on request. No installer carries any of them.

## The pin, and the CVE fix it must contain

The pin is `4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad` on upstream's master
("cli : fix stdout of model on windows (#4146)", 2026-11-03, ggml 0.28.0), the
commit every design measurement used (DD-3 in
[the implementation plan](../../docs/dictation/implementation-plan.md)).

Every pin must contain `60c0be6ac8fa71b1a2ae2dd938a31a34a508e774`, "whisper :
add check for ttype in whisper and parakeet (#4091)" (2026-10-02), the model
loader fix for CVE-2026-110810. `pin.json` records it in `contains`, and the
guard refuses a pin without it.

**Ancestry record (2026-11-05).** Checked with GitHub's compare API:

```sh
gh api repos/ggml-org/whisper.cpp/compare/60c0be6...4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad \
  --jq '{status, ahead_by, behind_by, merge_base: .merge_base_commit.sha}'
```

It answered `status: "ahead"`, `ahead_by: 399`, `behind_by: 0`, and a merge
base of `60c0be6ac8fa71b1a2ae2dd938a31a34a508e774`, the fix itself. A merge
base equal to the fix means the fix is an ancestor of the pin, the same fact
`git merge-base --is-ancestor 60c0be6 4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad`
proves in a full clone.

The tree was vendored from
`https://github.com/ggml-org/whisper.cpp/archive/4da27c41cebadb3cbbcd2563e9c7cbc8d84bdcad.tar.gz`;
the refresh read the commit id git embeds in that archive and matched it to the
pin before writing a file.

## Guards

| Guard | What it checks | Runs in |
|---|---|---|
| `scripts/check-vendored-whisper-cpp.sh` | the manifest both ways (no file missing, changed or unrecorded), every file inside the subset, no file gitignored, every file under `source/` exempt from end-of-line conversion, no symlink or upstream git settings file, `pin.json`'s fields and its `ggml` against `source/ggml/CMakeLists.txt`, `patches/` and `LICENSE` | `rust.yml` `guards`, `just check-everything` |
| `scripts/check-vendored-whisper-cpp.sh --self-test` | that an edited, missing, extra or `.gitignore` matched file fails, and every other refusal above | `rust.yml` `guards` |
| `scripts/check-dictation-fixtures.sh` | at most 2.75 MiB of fixtures in all, 1 MiB per file, 0.75 MiB of audio | `rust.yml` `guards`, `just check-everything` |
| `scripts/check-dictation-fixtures.sh --self-test` | that a 1.1 MiB dummy fixture fails and the real four pass at 2,399,463 B | `rust.yml` `guards` |
| `scripts/check-no-goose-block.sh` | the retired name guard reads this tree like every tracked file; it found nothing here at the pin | `rust.yml` `guards`, `just check-everything` |

`.gitattributes` marks `vendor/whisper.cpp/source/** -text` and `*.wav` and
`*.flac` binary, so a Windows checkout cannot rewrite a byte the manifest
hashed.

## Changing the pin

A pin moves only in a pull request of its own. The whole procedure, with the
builds, self tests and accuracy gates it needs, is "Changing the engine pin" in
[testing and release](../../docs/dictation/testing-and-release.md). The
vendoring part:

1. Prove the new commit contains every id in `pin.json`'s `contains`, starting
   with `60c0be6`: in a full clone (never shallow),
   `git merge-base --is-ancestor 60c0be6 <new commit>`, or the compare call
   above with the new commit, which must answer `ahead` or `identical`. Stop if
   either fails.
2. Edit `pin.json`: `commit`, `date` (the commit's date) and `ggml` (from the
   new `ggml/CMakeLists.txt`). Keep `contains`.
3. Download the archive of the new commit and vendor it:

   ```sh
   curl -L --fail -o /tmp/whisper.tar.gz \
     https://github.com/ggml-org/whisper.cpp/archive/<new commit>.tar.gz
   scripts/check-vendored-whisper-cpp.sh --refresh /tmp/whisper.tar.gz
   ```

   `--refresh` also takes a clone that has the commit. It refuses an archive
   made from any other commit, replaces `source/` with the subset, rewrites
   `source-manifest.json` and runs the full check, which fails if `ggml` in
   `pin.json` disagrees with the new tree. Copy the new `source/LICENSE` over
   `LICENSE` if it changed.
4. Run `scripts/check-no-goose-block.sh`, both guards and their self tests,
   and read the diff of `source/`: it is the change Biorouter is about to
   compile.
5. Update the whisper.cpp entry in this folder's `NOTICE` and in the
   repository's `NOTICE`.

## Patches

`patches/` is empty in v1. If upstream ever needs a change, it goes there as a
`*.patch` file, and the build applies the patches in lexical order to its copy
of the tree in `OUT_DIR`, never to `source/`. Keeping `source/` pristine is what
makes a pin change a wholesale replacement: a patch that stops applying is a
real conflict, not a merge of Biorouter's edits with themselves.
