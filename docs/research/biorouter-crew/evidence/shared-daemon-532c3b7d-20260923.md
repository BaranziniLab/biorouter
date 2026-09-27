# Shared-daemon acceptance on 532c3b7d

All cases use synthetic data and the immutable native non-test debug pair built
from `532c3b7d954928b3ace92495d7a3e159efae6b9c`:

- CLI SHA-256: `edc9a59ba6afbc37df6b19d09070b3c931f4bd2a6de91f5f3c7965bd165efdef`
- Daemon SHA-256: `4a1082ec9e1e0cc0b464c8cd608156a21cd24544c27cd87a2d42e7f84b16185c`
- Both mode 0555, ARM64 Mach-O, version 1.91.1.

Alice, Bob and Carol use fresh isolated profiles and real Linux UIDs 1101,
1102 and 1103. Daemons were restarted with their intended profile SSH
environment, and users reauthenticated and unlocked through supported flows.
The rootless broker hash remains
`4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.
Task-owned legacy Crew Electron processes were closed and absence was checked
before these acceptance runs. Unrelated applications remained open.

## Paused observer, fairness and released slots

The earlier 5455 test had acknowledged 60 messages of approximately 50 KiB each,
but its paused reader received only 12 before an unexpected EOF. With the 532c
pair, Alice observed the same retained backlog from its original opaque cursor,
paused reading for six seconds, then resumed. The stream contained exactly
60 expected message occurrences with 60 unique IDs, zero duplicate IDs and no
missing marker indices. There were 90 total stream lines and no observer error.
Bob's independent post/read completed in 0.618/0.026 seconds, both below the
predefined three-second bound.

With simultaneous paused and consuming observers, the consuming observer
recovered all 60 expected markers; Bob's post/read completed in 0.516/0.028
seconds. After the paused observer was terminated, a replacement was admitted
immediately and recovered all 60 markers.

Additional large mutations were refused because projected logical state would
exceed the broker's 16 MiB quota. The replay reused acknowledged messages;
small responsiveness posts remained available. No quota override, pruning or
registry mutation was used.

## Membership and derived-source revocation

A direct membership case queued three pre-revocation markers, removed Bob from
the channel, then admitted two owner posts. Bob received the three previously
authorized markers, zero new markers, and an error-bearing terminal result.

For a distinct provenance case, Bob joined restricted source A and destination
B. A session grant selected B and explicitly included A as context. A private,
loopback deterministic provider called actual Crew tools: `context.manifest`
returned A's canary, and `run.project` posted a derived result to B. Removing
Bob only from A made his next context turn fail with
`grant_expired: run revoked, expired or policy changed` before any provider
request. The provider audit delta was zero and the canary was absent from the
failed output. Bob retained B membership.

A positive visibility control re-invited Bob into A. B history then contained
one instance of derived message `b31210b8-aecc-4cbb-82d1-1d98f02f4474`. Removing
Bob only from A again hid that message, while B-only human control message
`fee9ad65-5245-435c-914d-7fd62c27b748` remained visible exactly once. This matches
the broker's requirement to retain access to every source of a derived message;
the earlier empty B page was expected filtering, not a membership defect.

These cases establish bounded observer and provider admission behavior. They
do not claim retroactive removal of bytes already delivered while authorized,
or a complete queued-derived-observer race matrix.

A later paused B observer started from a latest-history cursor after the
derived message, so that message was not eligible for stream delivery. Of
three B-only controls sent before A revocation and two afterward, the resumed
reader received one earlier frame, then terminated with an error indication.
The harness did not retain the exact error frame/code. A fresh B-only control
remained visible in Bob's direct history, but a bounded reconnect-stream read
yielded no frame. This attempt establishes neither queued-derived exclusion
nor reconnect catch-up; those cases remain unqualified. The earlier frame may
already have been in the OS pipe when membership changed.

## Interactive terminal continuation recovery

A separate fresh profile and loopback synthetic streaming provider exercised
ordinary daemon admission and an actual terminal client. Noninteractive pending
resume refused before dispatch. Interactive leave preserved the claim, abandon
resolved it, and takeover admitted exactly one explicit successor input whose
response persisted in the authoritative session. Final resume showed no active
turn or pending continuation. The [validation report](../validation-report.md)
records the focused tests and ordinary API flow.

## Three-user natural-model and file workflow

Bob's ordinary shared-daemon session `20260923_3` used the installed local
Ollama endpoint and `qwen3:8b`, with an explicit destination/context grant.
Actual `crew__connections`, `messages.history` and `run.project` calls read
seeded fact `549cb19a-925d-450d-b1e0-9795f12284ae` and posted progress receipt
`0f75d697-a608-408c-aca2-20f3f2c30f50`. Alice and Carol independently fetched
the destination and each found the fact and model result exactly once. The
stream finished normally. This used the installed model, separately from the
deterministic provider fixture used for precise revocation assertions.

The attachment upload initially returned transfer
`17a75769d6ac4fb286582d9d0ca3f960` in `starting` state, without a blob ID.
Using that transfer ID prematurely was refused as `attachment unavailable`.
The supported `files watch` flow reached `completed` and returned blob
`4465105b-6a1e-4a51-bade-b8240f0d26ef`. Alice posted attachment message
`66f51992-399d-4f9f-8e0f-2a48f43d5627`; Bob and Carol independently read it,
downloaded the completed blob and waited for their download transfers to
complete. Both files matched the original SHA-256:
`4fd7fd53f52ec3ba906e93b9f6f4ecf7690c1e7b8f703cb5c3836952f3fee4cf`.
This qualifies the bounded three-user native CLI attachment workflow; native
graphical Save dialogs remain a separate acceptance gate.

## Remaining scope

The [Linux report](linux-532c3b7d-20260923.md) records the same source's ARM64
build, focused regressions and ordinary-UID lifecycle. Mixed GUI/CLI, native Save,
AWS product acceptance and native Windows runtime remain separate gates. Earlier
5455 natural-model evidence keeps its original artifact scope.
