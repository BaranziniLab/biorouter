#!/usr/bin/env bash
# Build the Fareez et al. 2022 subset M-22 and Tier 2 read (O-38): simulated clinical
# interviews, CC0 on their figshare article, cut into 2 to 25 s utterances that name drugs.
#
# Run on the maintainer's machine, never in CI; the audio, the transcripts and the
# alignments it makes are never committed or cached. What is committed: this script,
# fareez_subset.py, fareez-terms.txt, the pins below, and the public manifest the run
# writes (clip names, times, hashes and term counts, no text), kept beside this script as
# fareez-subset.manifest.jsonl. The first build (2026-11-06, the 24 interviews that name
# the most terms, macOS with Metal, ffmpeg 9.0.2) cut 80 clips holding 104 term
# occurrences. A later build writes its own public manifest to compare with that file; the
# FLAC hashes can differ with another ffmpeg, the clip names, times and terms should not.
#
#   scripts/dictation/fetch-fareez-subset.sh <work folder> [biorouter binary]
#
# Steps, each stopping the run when it fails:
#   1. download Data.zip through the figshare API's own download URL (the web path answers
#      a scripted client with an AWS WAF challenge); an existing copy is resumed or reused;
#   2. read the figshare article's license and stop unless it is CC0;
#   3. check the archive's size and SHA-256 against the pins;
#   4. unpack it and cut the subset with fareez_subset.py, which aligns each interview with
#      Biorouter's own turbo model (`biorouter transcribe`, a catalog model pinned by its
#      SHA-256; it must be installed) and ffmpeg;
#   5. write the manifest with each clip's SHA-256.
#
# Point BIOROUTER_DICTATION_FAREEZ_DIR at <work folder>/subset to run M-22 with it.
set -euo pipefail

ARTICLE_ID=16550013
FILE_ID=30598530
ZIP_SIZE=1034153474
# Recorded at the first download, 2026-11-06 (figshare's MD5 9c79f2050dbdaf13fb1c2c5d38587d60).
ZIP_SHA256=20ef65540768d49ab6368672994aaef6acb21117d139e5d91e76a32432da951e
DOWNLOAD_URL="https://ndownloader.figshare.com/files/$FILE_ID"
ARTICLE_URL="https://api.figshare.com/v2/articles/$ARTICLE_ID"
MAX_INTERVIEWS="${FAREEZ_MAX_INTERVIEWS:-24}"

if [ $# -lt 1 ]; then
  sed -n '2,25p' "$0"
  exit 2
fi
work="$1"
biorouter="${2:-biorouter}"
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$work"

for tool in curl unzip ffmpeg python3; do
  command -v "$tool" >/dev/null || { echo "fetch-fareez-subset: $tool is needed" >&2; exit 1; }
done

# 2 first: there is no point downloading a gigabyte whose license is no longer CC0.
license=$(curl -fsS --max-time 60 "$ARTICLE_URL" | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["license"]["name"])')
echo "figshare article $ARTICLE_ID license: $license" | tee "$work/license.txt"
if [ "$license" != "CC0" ]; then
  echo "fetch-fareez-subset: the license is $license, not CC0; stopping" >&2
  exit 1
fi

zip="$work/Data.zip"
if [ ! -f "$zip" ] || [ "$(wc -c < "$zip" | tr -d ' ')" != "$ZIP_SIZE" ]; then
  curl -fL --retry 3 -C - -o "$zip" "$DOWNLOAD_URL"
fi
size=$(wc -c < "$zip" | tr -d ' ')
if [ "$size" != "$ZIP_SIZE" ]; then
  echo "fetch-fareez-subset: Data.zip is $size bytes, not $ZIP_SIZE; stopping" >&2
  exit 1
fi
digest=$(shasum -a 256 "$zip" | cut -d' ' -f1)
if [ "$digest" != "$ZIP_SHA256" ]; then
  echo "fetch-fareez-subset: Data.zip has SHA-256 $digest, not the pinned $ZIP_SHA256; stopping" >&2
  exit 1
fi

unzip -q -o "$zip" -d "$work/raw"
python3 -I "$here/fareez_subset.py" "$work/raw/Data" "$work/subset" "$biorouter" "$MAX_INTERVIEWS"
echo "subset: $work/subset (public manifest: $work/subset/manifest.public.jsonl)"
