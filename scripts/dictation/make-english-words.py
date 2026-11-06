#!/usr/bin/env python3
"""Build crates/biorouter-dictation/assets/english-words.zst, dictation's English word list.

What the list is for (docs/dictation/dictionary.md, "The English word list"): the
`collides` flag, the decoder bonus's start piece filter, the single token guard of the
exact key matcher, the fuzzy matcher's English guard and the wordy alias rule. A system
word list is never used: it lacks `coding`, `here's`, `aren't`, `encodes` and `cells`,
and Windows has none.

The source is one export of the English Speller Database (ESDB, previously SCOWL) from
https://app.aspell.net/create with size 60, spellings US GB CA AU, variant level 6, the
hacker and roman numeral special lists, and both diacritic forms. The export is pinned by
its SHA-256 below; a different file stops the build, so the committed list is reproducible
from the pinned file alone. ESDB's export page is versioned by its git revision, which the
export's own header records.

The list is every word of the export lowercased, without any word the retired name guard
(scripts/check-no-goose-block.sh, R-31) matches, plus the hand list below, one word per
line in code point order. A word the export lists only capitalized (a proper noun or a
brand: `Boston`, `Ventolin`) is marked with a leading `+`: it is English for the guards
that keep correction away from ordinary speech, but a dictionary term spelled like it does
not `collide`, because nobody says `Ventolin` meaning anything else. A word listed in
lower case or in capitals (`spoke`, `CD`) is unmarked. The guard's pattern is read from the
guard itself, so this script never spells it. The export's copyright and permission notices
travel inside the list as `#` lines, because the license asks for them in all copies; the
loader skips those lines.

Usage (Python 3.14 or later, which has compression.zstd; standard library only):

    python3 -I scripts/dictation/make-english-words.py <esdb-export.txt>
"""

import hashlib
import pathlib
import re
import sys

from compression import zstd

SOURCE_SHA256 = "c500ee2f68e7254e07c4d90abdc2c8d00c146a5cf1f812108423bb6bbc71993a"
SOURCE_PARAMETERS = "size 60, spelling US GB CA AU, variant level 6, special hacker roman-numerals, diacritics both"
# Abbreviations dictation must treat as English (dictionary.md): the near collisions the
# measurements found (UCSD beside UCSF, CRISPR beside CRISPRi, PhD beside PHD2).
HAND_LIST = ["phd", "uc", "ucsd", "crispr", "rna", "dna", "pdf"]
ZSTD_LEVEL = 19

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
GUARD = ROOT / "scripts" / "check-no-goose-block.sh"
OUT = ROOT / "crates" / "biorouter-dictation" / "assets" / "english-words.zst"


def guard_pattern() -> re.Pattern:
    """The guard's case insensitive pattern, from its `ci='...'` line."""
    for line in GUARD.read_text(encoding="utf-8").splitlines():
        if line.startswith("ci='") and line.endswith("'"):
            # awk and grep read it as an extended regex; Python reads it the same way for
            # the alternatives and classes it uses.
            return re.compile(line[4:-1].replace("[[:alnum:]_-]", "[A-Za-z0-9_-]"), re.IGNORECASE)
    sys.exit(f"no ci= pattern in {GUARD}")


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    source = pathlib.Path(sys.argv[1])
    raw = source.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != SOURCE_SHA256:
        sys.exit(f"{source} has SHA-256 {digest}, not the pinned {SOURCE_SHA256}; stopping")
    text = raw.decode("utf-8")
    header, sep, body = text.partition("\n---\n")
    if not sep:
        sys.exit("the export has no --- line between its header and its words")

    pattern = guard_pattern()
    # folded word -> whether some listing of it is common (lower case or capitals)
    words = {}
    removed = 0
    for line in body.splitlines():
        listed = line.strip()
        word = listed.lower()
        if not word or word.startswith("#") or word.startswith("+"):
            continue
        # The guard removes these tokens before matching, so they are allowed.
        if pattern.search(word.replace("mongoose", "")):
            removed += 1
            continue
        common = listed == word or (listed == listed.upper() and any(c.isalpha() for c in listed))
        words[word] = words.get(word, False) or common
    for word in HAND_LIST:
        words[word] = True

    notice = [
        "# Biorouter's dictation word list, derived from the English Speller Database.",
        f"# Source parameters: {SOURCE_PARAMETERS}.",
        f"# Source SHA-256: {SOURCE_SHA256}.",
        "#",
    ] + ["# " + line if line else "#" for line in header.splitlines()]
    lines = [w if words[w] else "+" + w for w in sorted(words)]
    listing = "\n".join(notice + lines) + "\n"
    packed = zstd.compress(listing.encode("utf-8"), level=ZSTD_LEVEL)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(packed)
    proper = sum(1 for common in words.values() if not common)
    print(
        f"{len(words)} words, {proper} of them proper ({removed} removed by the retired name guard), "
        f"{len(listing.encode('utf-8'))} bytes, {len(packed)} compressed, "
        f"sha256 {hashlib.sha256(packed).hexdigest()}"
    )


if __name__ == "__main__":
    main()
