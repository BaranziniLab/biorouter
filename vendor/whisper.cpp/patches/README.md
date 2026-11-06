# whisper.cpp patches

Empty in v1: Biorouter builds whisper.cpp exactly as upstream wrote it at the
commit `../pin.json` names.

If a change to upstream is ever needed, it goes here as a `*.patch` file, and
the build applies the patches in lexical order to its own copy of the tree in
`OUT_DIR`, never to `../source/`. The vendored source stays byte identical to
upstream, so moving the pin replaces `source/` wholesale and a patch that no
longer applies is a real conflict.

`scripts/check-vendored-whisper-cpp.sh` refuses any other kind of file in this
folder.
