# Post-fix Crew NDJSON compression evidence

The owned disposable ACL Alice daemon was stopped and restarted through the
supported `biorouter crew daemon stop` and `daemon start` commands, using
the existing mode-600 fixture approval input. It was then reconnected through
the supported `biorouter crew --connection ... connect` flow. This test did not
modify the GUI profile; actual renderer replay remains a separate pending check.

Binary used:

- `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd`
- SHA256 `0890a909cd1e6933ae6aa06549275e5d0152a72c006b21a7bb713de0c7ee2cf6`
- Runtime PID `71405`

The sanitized probe is `/private/tmp/crew-live-luna/postfix_acl_compression_probe.py`
and its receipt is `/private/tmp/crew-live-luna/postfix_acl_compression_receipt.json`.
It reads the daemon descriptor and connection ID programmatically, keeps secrets
in memory, and prints no credential or payload content.

Post-fix live result:

- `workspace.snapshot`: HTTP 200, JSON, 2,230 bytes, headers in 60.8 ms.
- `observe` with `Accept-Encoding: gzip`: HTTP 200, `application/x-ndjson`, no
  `Content-Encoding`, chunked transfer; first decoded frame was `type=state`,
  2,352 wire bytes, in 2,390.2 ms while the stream remained open.
- The positive snapshot and observer used the real ACL workspace and saved
  connection; no daemon state was manually edited.

Before the fix, the incremental gzip receipt is
`/private/tmp/crew-live-luna/gzip_probe_incremental_receipt.json`: HTTP 200 with
`Content-Encoding: gzip`, only the 10-byte gzip header arrived at 2,551.1 ms,
and no decoded NDJSON frame arrived within 10 seconds.

Focused production-helper command:

```text
CARGO_BUILD_JOBS=2 cargo test -p biorouter-server --bin biorouterd compression_tests -- --nocapture
```

Result: 3 passed, 0 failed, 733 filtered out. The tests cover valid large JSON
gzip, an open NDJSON stream's first frame, and an open SSE stream's first frame.
Regression source SHA256:
`32f081bad2bdddde72b028a54197aa425d77450f6eee2e2009c7a268bd6152f3`.

Required checks:

- `CARGO_BUILD_JOBS=2 cargo build -p biorouter-server --bin biorouterd`: passed.
- `CARGO_BUILD_JOBS=2 just check-everything`: passed, exit 0.
- `CARGO_BUILD_JOBS=2 just generate-openapi`: passed; generated schema and
  frontend API were unchanged (`git status` showed no schema/API diff).

The pre-fix control used the GUI fixture; the post-fix HTTP check used the separate
owned ACL fixture. These results qualify the daemon stream, not three-user GUI
collaboration or native Save. No new authentication mechanism was introduced.
