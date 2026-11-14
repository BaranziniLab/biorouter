# Voice dictation research

This folder holds external research for Biorouter's local voice dictation. It studies other dictation applications, speech engines and speech models, and it records the measurements Biorouter ran while choosing an engine, a model catalog and a dictionary design. The documents describe systems outside Biorouter and link none of Biorouter's code, so they stay true while Biorouter changes. They do age against their subjects, and each one states the date its facts were taken. Biorouter's own design, and the plan that acts on this research, live in [voice dictation](../../dictation/README.md).

## Documents

| Document | What it covers |
|---|---|
| [Dictation tools survey](dictation-tools-survey.md) | How whisper.cpp, sherpa-onnx, transcribe.cpp and other engines, and dictation apps such as Handy, VoiceInk, OpenWhispr and OpenAI Codex, capture speech, run local models, apply a personal vocabulary and ship their models. It also covers the speech models beyond Whisper. A snapshot as of 2026-11-04. |
| [Speech model benchmarks](speech-model-benchmarks.md) | Biorouter's own measurements of speech engines and models on biomedical dictation, with and without dictionary biasing, and the evaluation material behind them. |

## Related documentation

- [Voice dictation](../../dictation/README.md): Biorouter's own dictation design and the plan that acts on this research.
- [Dictation engine and models](../../dictation/engine-and-models.md): the engine and models Biorouter chose from the options studied here.
- [Dictation dictionary](../../dictation/dictionary.md): the personal dictionary design that the vocabulary findings feed.
- [Research index](../README.md): the other external studies Biorouter keeps.
