# Which session the app's hooks are installed on

> **What this is.** Why the Electron main process installs its permission handlers, its CSP response header and its proxy on the renderer's `persist:` partition as well as on the default session, what each of those does now that it governs the app's own windows for the first time, and the one hook that deliberately stays behind.
> **Status:** Current; measured 2026-09-10 on Electron 39.8.10.
> **Audience:** contributors touching `ui/desktop/src/main.ts`, the renderer CSP, or anything that creates a window or a `Session`.

## The rule

Every hook the app installs on a `Session` goes on **every session the app's own
windows render in**. Today that is `session.defaultSession` and the partition named by
`RENDERER_PARTITION`, and `appSessions()` in `ui/desktop/src/main.ts` is the single list
both the installer and the proxy configuration read.

A `persist:` partition is a different `Session` object. A hook on `defaultSession` does
not reach it, and — this is the part that makes the failure silent — **an unhandled
session grants by default**: a view whose partition has no permission handler returns
`granted` for notifications and allows geolocation while the app's handler is never
consulted.

## What was wrong

Both Biorouter windows — the main window and the launcher — are created with
`partition: 'persist:biorouter'`. Every hook was installed on `session.defaultSession`:
both permission handlers, the CSP response header, the proxy, and the `Origin` rewrite.
So none of them governed the window the user was looking at.

The only window left on the default session is the drag ghost, which loads a `data:` URL
with no preload, no node integration and DevTools off. `onHeadersReceived` does not fire
for a `data:` URL at all. Four security hooks were installed on a session whose sole
content is a transparent drag chip.

Two things kept it invisible:

- **The tests pinned the policy strings.** `frameSrcCsp.test.ts` and
  `workspaceChannelCsp.test.ts` assert what the two CSP policies *say*. A string pin
  cannot tell you which policy is *live*.
- **The comments asserted the opposite.** Three places — `main.ts`, `index.html` and
  `workspaceChannelCsp.test.ts` — said "both policies apply to this window and the
  stricter wins". That was aspirational, and a comment is not a mechanism.

It was caught by measurement, not by reading. A deliberate `frame-src` violation in the
running app fired **exactly one** `securitypolicyviolation`, whose `originalPolicy` was
byte-for-byte the `<meta>` tag. Two enforcing policies produce two events.

`utils/embeddedBrowser.ts` had already measured and written down the same fact for its
own partition. The app's own windows were the ones nobody had checked.

## Why not `app.on('session-created')`

It fires for every session this process creates. That includes the live browser's
`persist:biorouter-embedded-browser` and each ephemeral `biorouter-managed-app-<uuid>`,
both of which install their own, different rules on purpose. Handing the embedded
browser `default-src 'self'` would break every page it exists to show. The app's policy
goes on the app's own sessions, by name.

## Ordering

The hooks are installed **before `appMain`'s first `await`**, and that is load-bearing.
`open-url` and the `.brxt` file handlers each do their own `await app.whenReady()` and
then call `createNewWindow`. Their continuations are queued after this module's, so
everything before the first `await` is guaranteed to run before any of them — and
nothing after it is. A permission handler installed after a window exists has already
missed that window's first document load.

## What changed when the header started applying

The header policy and the `<meta>` policy both enforce, and a document under two
policies gets their **intersection**. Making the header live therefore changes the
effective policy wherever the two disagreed. Each divergence was measured on Electron
39.8.10, in both the packaged (`file://`) and dev (`http://localhost:517x`) document
shapes.

| Divergence | Measured | Decision |
|---|---|---|
| `script-src`: meta grants `'unsafe-inline'`, header did not | `onHeadersReceived` **does** fire for a `file://` document; without the token the inline script in `index.html` does not run | Header gains `'unsafe-inline'` |
| `upgrade-insecure-requests`: header only | `fetch('http://127.0.0.1:…')` returns 200 and `new WebSocket('ws://127.0.0.1:…')` reaches the server, both unupgraded — loopback is already potentially trustworthy | Kept; inert for the daemon |
| `connect-src`: meta has blanket `https: wss:`, header has a maintained allowlist | The renderer's only socket is the loopback workspace channel; nothing in `src/` opens a non-loopback `fetch`, `EventSource` or `WebSocket` (everything external goes through main-process IPC) | Header's narrower list becomes effective — a real tightening |
| every other directive | identical in both | unchanged |

The `script-src` one is the hazard worth understanding. `index.html` opens with an
inline `<script>` — the pre-hydration theme-family boot that sets `data-theme` before
first paint. The meta policy grants `'unsafe-inline'` for it; the header did not, and
while the header reached no window that cost nothing. The moment it reached the
renderer, the missing token would have become the binding one and the boot script would
have died — along with vite's react-refresh preamble, which is also inline.

⚠ Adding the token to the header does **not** widen the effective policy. The meta has
always permitted inline script and has always been enforced, so the intersection is
unchanged; what changed is that the header now agrees with it. Tightening it for real
means removing the token from *both* files and giving the boot script a hash — a
separate change, with its own measurement.

## The one hook that stays on the default session

`onBeforeSendHeaders`, which rewrites every request's `Origin` to the vite dev origin.
It has been there since the initial commit, inherited from upstream, and it is
deliberately **not** extended to the renderer's partition:

- **The renderer does not need it.** A packaged `file://` document sends no `Origin` on
  `fetch`, and Electron does not CORS-check a `file://` initiator — measured: a fetch to
  loopback returns 200 with no `Access-Control-Allow-Origin` in the response at all. Its
  WebSocket sends `Origin: file://`, which `routes/workspace.rs` admits by name, and the
  dev renderer sends `http://localhost:517x`, which `routes::is_local_origin` admits.
  Every gate already passes on the renderer's real origin.
- **Extending it would be strictly worse.** The daemon's socket gates are same-origin
  tests (`origin_matches_host`) against the browser-set `Origin`. Replacing that with a
  constant the main process invented means the check validates our own literal instead
  of the renderer's identity.

`installDefaultSessionOnlyHooks` is where it lives, and
`src/rendererSessionHooks.test.ts` carries it as the one documented exception to hook
twinning — so a *new* default-session hook fails that test rather than quietly joining
it.

## What the tests assert

`src/rendererSessionHooks.test.ts` reads `main.ts` as text, with the same
comment-stripping discipline the two CSP tests use, because the fact under test is which
`Session` object a hook was attached to and there is no Electron in vitest. It fails
when: the partition is declared more than once, a window names a partition literal
instead of the constant, `appSessions()` stops covering both, the installation drifts
after `appMain`'s first `await`, a `defaultSession` hook appears with no partition twin
and no documented exemption, or someone reaches for `app.on('session-created')`.

`src/frameSrcCsp.test.ts` keeps its `frame-src` pins and adds the `script-src`
agreement, including a non-vacuity check that `index.html` really does still open with
an inline script.

## Related documentation

- [Where a generated artifact is displayed](artifact-display-surfaces.md) — the panel these policies govern, and the second CSP that was removed with the inline renderer.
- [How an Auto Visualiser figure's libraries reach the renderer](artifact-cdn-assets.md) — the other half of the artifact CSP story, where the main process inlines a pinned CDN script before the policy applies.
- [The preview panel](preview-panel/README.md) — where the "an unhandled session grants by default" measurement was first made, for the embedded browser's partition.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — how to get a running app to re-measure any of this in.
