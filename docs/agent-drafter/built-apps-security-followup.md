# Built apps security and lifecycle followup

Worktree: `/Users/wgu/Desktop/BioRouter/.claude/worktrees/built-apps-security-followup`.
Branch: `codex/built-apps-security-followup`.
Baseline: the original hardening handoff branch, including interrupted fixes already committed there.

The review input is `/Users/wgu/biorouter-runs/built-apps-qa/notes/fix-input-security-lifecycle.json`.
Its handoff status predates several fixes present in the baseline. Those implementations were
reviewed before adding the changes below.

## Finding disposition

| Finding | Resolution and regression evidence |
| --- | --- |
| HTTP LAN app/shell isolation fails open (high and duplicate medium finding) | Baseline refused app browser credentials on insecure network hosts. This followup also refuses the secret-bearing shell and its token exchange, because old app tabs can survive an update. Nonloopback forwarded HTTPS additionally requires browser-controlled Fetch Metadata; a forged `X-Forwarded-Proto: https` alone is refused. Tests cover app surfaces, forged proxy headers, shell cookie/token refusal, and positive modern TLS proxy navigation. |
| Desktop preview cancels figure-frame | Owned by the desktop lane in `managedAppPreviewPolicy.ts`. Integration must include its change and policy tests. This lane checked the figure document has its own sandbox/CSP and the SDK uses that route. |
| Figure reload receives no second payload | Already fixed in baseline SDK and harness: every ready message from the current frame window receives the figure; messages from another window receive nothing. The SDK harness passed this regression in this worktree. |
| Deleted app drops cached agent's UiBridge | Baseline retains the bridge. This followup adds generation-safe Drop cleanup, including interruption during setup/KB/consult awaits. Tests prove a retained bridge can reconnect and cleanup from an older connection cannot detach its replacement. |
| Factory reset bypasses delete hook | Baseline uses `ArtifactStore::delete_all`. Reset now returns the number actually removed, including broken and unfinished apps. The regression includes healthy, malformed-manifest and unfinished directories and checks every hook callback. |
| Arbitrary page-token checks grow cache | Baseline uses lookup-only token comparisons. Its socket retirement cache still grew for unknown ids, so tokens and socket lifetimes now share one cache, and unknown socket ids return an already-cancelled lifetime without insertion. A regression probes 128 unknown ids. |
| Case variant delete forgets wrong id | Baseline refuses non-exact directory names for delete. This followup also rejects case-variant existing paths for reads, writes and existence checks, preventing alias launch cookies or assets from outliving canonical deletion. The store test validates exact-case read access and delete hooks. |
| Deleted socket keeps replacement workspace access | Baseline cancellation was observed only in two event loops. This followup wraps the whole socket task, cancels turns through child tokens, and detaches its bridge on every exit. Page tokens are also bound to `app_uid`, preventing a late response from a deleted manifest from authenticating a replacement. Tests cover a parked operation outside event loops, lifetime cancellation, and late old-generation page tokens. |

The build lane also identified SDK `br.run` tool/error strings being interpolated into HTML.
Both now pass through `escapeHtml`. The SDK harness includes malicious `<img onerror>` strings
and proves no element is injected while the literal message remains visible.

Independent desktop review found a second reconnect race inside `UiBridge::detach`: the old
connection checked its atomic generation before locking the sender, so a new attach could install
its channel in between and then lose that channel and its parked asks. Attach and detach now hold
one generation mutex throughout their sender/reset/cancellation work. A deterministic concurrent
regression parks detach at the sender lock while another thread reattaches, then proves the new
channel and its UI request remain live. The shell also uses the real request URI when checking
HTTP/2 authority-only requests; a network authority cannot be mistaken for an absent browser host.

## Verification

- `git diff --check`: passed.
- `rustfmt --edition 2021` on the owned Rust files: passed.
- `scripts/agent-drafter/ui-control-harness.mjs`: **ALL PASS**, including repeat-ready figure
  handshake, height handling, malicious tool name and malicious error text. Uses the repository's
  esbuild and jsdom dependencies, with an isolated loopback test port; the harness exited cleanly.
- Cargo compilation and Rust test execution are intentionally delegated to the primary agent's
  serial integration gate to avoid parallel builds on the low-free-disk machine. They are pending
  at this checkpoint. Required focused commands:

```sh
cargo test -p biorouter-server --lib app_isolation_and_lifecycle_tests
cargo test -p biorouter-server --lib auth::
cargo test -p biorouter-server --lib routes::web_ui::
cargo test -p biorouter-server --lib routes::reset::
cargo test -p biorouter-mcp --lib agent_drafter::store::
cargo test -p biorouter-mcp --lib agent_drafter::control::
```

The integration gate must include the full clippy/test checks and real-app preview/figure/socket
verification. This checkpoint does not claim Rust tests or live Electron acceptance have passed.

## Preserved limits

Per-app origins remain the planned complete isolation boundary. Legacy browsers that send no
Fetch Metadata are still accepted on loopback for non-browser compatibility and therefore are
not protected by the navigation gate there; such browsers are refused through nonloopback HTTPS
proxies. Browsers without `noopener-allow-popups` support and shared origin storage remain
documented limits. No credential contents were read or printed by this lane.
