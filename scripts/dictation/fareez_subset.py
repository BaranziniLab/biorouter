#!/usr/bin/env python3
"""Cut the Fareez et al. 2022 subset (O-38, M-22) from the unpacked interviews.

Called by `fetch-fareez-subset.sh` after the download, the license check and the hash
check. Standard library only; it reads the archive's files by path and runs nothing it
downloaded.

The transcripts carry no timestamps, so each interview is aligned first. The aligner is
Biorouter's own: `biorouter transcribe --format json --no-dictionary` with the catalog's
`whisper-large-v3-turbo-q5_0`, whose file the catalog pins by SHA-256, gives segments of at
most 25 s with their times, and the human transcript is then aligned to those segments word
by word (`difflib.SequenceMatcher`, no junk heuristic). A segment becomes a clip when both of
its ends land on matched words, it lasts 2 to 25 s, and the human words it covers name a
term of `fareez-terms.txt`. The clip's reference is the human transcript's words, never the
recognizer's. Clips are cut with ffmpeg to 16 kHz mono FLAC, 0.2 s of padding on each side.

Writes, in the output folder: the clips, `manifest.jsonl` (`clip`, `text`, `terms`,
`source`, `t0`, `t1`, `flac_sha256`; read by `dictation_accuracy.rs`), `terms.json`, and
`manifest.public.jsonl`, the same rows without `text`, which is the file that may be
committed. A reference whose text trips the retired name guard is left out and reported.

Usage: python3 -I fareez_subset.py <unpacked Data folder> <output folder> <biorouter binary>
       [max interviews, default 24]
"""

import difflib
import hashlib
import json
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
MODEL = "whisper-large-v3-turbo-q5_0"
PAD_S = 0.2
MIN_S, MAX_S = 2.0, 25.0


def terms() -> list:
    lines = (HERE / "fareez-terms.txt").read_text(encoding="utf-8").splitlines()
    return [t.strip() for t in lines if t.strip() and not t.startswith("#")]


def guard_pattern() -> re.Pattern:
    for line in (ROOT / "scripts" / "check-no-goose-block.sh").read_text(encoding="utf-8").splitlines():
        if line.startswith("ci='") and line.endswith("'"):
            return re.compile(line[4:-1].replace("[[:alnum:]_-]", "[A-Za-z0-9_-]"), re.IGNORECASE)
    sys.exit("the retired name guard's pattern was not found")


def count(text: str, term: str) -> int:
    return len(re.findall(r"(?<![a-z0-9])" + re.escape(term.lower()) + r"(?![a-z0-9])", text.lower()))


def norm(word: str) -> str:
    return re.sub(r"[^a-z0-9']", "", word.lower())


def human_tokens(path: pathlib.Path) -> list:
    """(normalized, as written) for every word of the transcript, speaker labels left out."""
    out = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = re.sub(r"^\s*[DP]:\s*", "", line)
        for token in line.split():
            n = norm(token)
            if n:
                out.append((n, token))
    return out


def transcribe(biorouter: str, mp3: pathlib.Path, cache: pathlib.Path) -> list:
    """The interview's segments, from the alignment cache when an earlier run made them."""
    if cache.exists():
        return json.loads(cache.read_text(encoding="utf-8")).get("segments", [])
    result = subprocess.run(
        [biorouter, "transcribe", str(mp3), "--format", "json", "--no-dictionary", "--model", MODEL],
        capture_output=True, text=True, stdin=subprocess.DEVNULL,
    )
    if result.returncode != 0:
        print(f"  {mp3.name}: transcribe exited {result.returncode}", file=sys.stderr)
        return []
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(result.stdout, encoding="utf-8")
    return json.loads(result.stdout).get("segments", [])


def main() -> None:
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    data, out, biorouter = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
    limit = int(sys.argv[4]) if len(sys.argv) > 4 else 24
    term_list = terms()
    guard = guard_pattern()
    out.mkdir(parents=True, exist_ok=True)

    # The interviews that name the most terms first, so a capped run keeps the most terms.
    ranked = []
    for txt in sorted((data / "Clean Transcripts").glob("*.txt")):
        text = txt.read_text(encoding="utf-8", errors="replace")
        n = sum(count(text, t) for t in term_list)
        mp3 = data / "Audio Recordings" / (txt.stem + ".mp3")
        if n and mp3.exists():
            ranked.append((-n, txt.stem, txt, mp3))
    ranked.sort()
    rows, dropped = [], 0
    for _, stem, txt, mp3 in ranked[:limit]:
        print(f"{stem}: aligning", file=sys.stderr)
        segments = transcribe(biorouter, mp3, out.parent / "alignment" / f"{stem}.json")
        human = human_tokens(txt)
        asr, owner = [], []
        for i, seg in enumerate(segments):
            for token in seg.get("text", "").split():
                n = norm(token)
                if n:
                    asr.append(n)
                    owner.append(i)
        matcher = difflib.SequenceMatcher(None, asr, [h[0] for h in human], autojunk=False)
        to_human = {}
        for block in matcher.get_matching_blocks():
            for k in range(block.size):
                to_human[block.a + k] = block.b + k
        for i, seg in enumerate(segments):
            idx = [k for k, o in enumerate(owner) if o == i]
            if not idx:
                continue
            first, last = idx[0], idx[-1]
            if first not in to_human or last not in to_human:
                continue
            h0, h1 = to_human[first], to_human[last]
            if h1 < h0 or not (0.5 <= (h1 - h0 + 1) / len(idx) <= 2.0):
                continue
            t0, t1 = float(seg["t0"]) - PAD_S, float(seg["t1"]) + PAD_S
            t0 = max(0.0, t0)
            if not (MIN_S <= t1 - t0 <= MAX_S):
                continue
            reference = " ".join(h[1] for h in human[h0:h1 + 1])
            found = {t: count(reference, t) for t in term_list if count(reference, t)}
            if not found:
                continue
            if guard.search(reference.lower().replace("mongoose", "")):
                dropped += 1
                continue
            clip = f"{stem}_{int(t0 * 1000):07d}.flac"
            # A segment Whisper timed past the end of the recording cuts nothing; it is
            # left out.
            cut = subprocess.run(
                ["ffmpeg", "-nostdin", "-loglevel", "error", "-y", "-ss", f"{t0:.3f}", "-to", f"{t1:.3f}",
                 "-i", str(mp3), "-ac", "1", "-ar", "16000", "-sample_fmt", "s16", str(out / clip)],
                capture_output=True,
            )
            if cut.returncode != 0 or not (out / clip).exists() or (out / clip).stat().st_size == 0:
                (out / clip).unlink(missing_ok=True)
                continue
            digest = hashlib.sha256((out / clip).read_bytes()).hexdigest()
            rows.append({"clip": clip, "text": reference, "terms": found, "source": mp3.name,
                         "t0": round(t0, 3), "t1": round(t1, 3), "flac_sha256": digest})
        print(f"{stem}: {sum(1 for r in rows if r['source'] == mp3.name)} clips", file=sys.stderr)

    with open(out / "manifest.jsonl", "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    with open(out / "manifest.public.jsonl", "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps({k: v for k, v in row.items() if k != "text"}, ensure_ascii=False) + "\n")
    occurrences = {}
    for row in rows:
        for t, n in row["terms"].items():
            occurrences[t] = occurrences.get(t, 0) + n
    (out / "terms.json").write_text(json.dumps(
        {"terms": sorted(occurrences, key=lambda t: -occurrences[t]), "occurrences": occurrences,
         "matching": "Each term is counted as a whole word, ignoring case, in the clip's reference text."},
        indent=2) + "\n", encoding="utf-8")
    print(f"{len(rows)} clips, {sum(occurrences.values())} term occurrences, "
          f"{dropped} left out by the retired name guard")


if __name__ == "__main__":
    main()
