import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

/**
 * BR-71 §4.3: the renderer opens ONE WebSocket per window to
 * `GET /ui/workspace` (`hooks/useWorkspaceChannel.ts`). It is the only
 * WebSocket the main renderer document opens — every other daemon call is
 * `fetch`/SSE over `http://127.0.0.1:<port>` — so it is the first request in
 * the app to need a `ws:` source in `connect-src`.
 *
 * CSP does NOT let an `http:` source expression cover a `ws:` URL. CSP3
 * §6.6.2.6 allows exactly three scheme relaxations — `http`→`https`,
 * `ws`→`wss`, and `ws`→`http`/`https` — and `http`→`ws` is not among them, so
 * `connect-src http://127.0.0.1:*` blocks `ws://127.0.0.1:62736/ui/workspace`
 * outright.
 *
 * That is not a theoretical reading. Measured in the dev GUI during Task 31's
 * live pass: the socket never connected, `securitypolicyviolation` fired with
 * `effectiveDirective: "connect-src"` and
 * `blockedURI: "ws://127.0.0.1:62736/ui/workspace?…"`, and every
 * `workspace_list` therefore reported `"gui_attached": false` while the GUI was
 * running in front of the user — which silently disables the whole GUI half of
 * the feature (no tabs open, `workspace_open` degrades, Decision 4's approval
 * guard mis-fires).
 *
 * Both policies have to allow it, because both apply to the window:
 *   - the `<meta http-equiv="Content-Security-Policy">` in `index.html`
 *   - the header `main.ts` attaches in `session.defaultSession.webRequest
 *     .onHeadersReceived` (built by `buildConnectSrc()`)
 * The most restrictive of the two wins, so allowing it in one and not the other
 * still blocks the socket. These assertions read the real shipped sources
 * rather than a copy.
 *
 * ⚠ "both apply to the window" was ASPIRATIONAL when this was written. Both
 * windows render in a `persist:` partition and the header was installed on
 * `session.defaultSession` — a different `Session` object — so until September
 * 2026 the meta tag was the only policy enforcing, and the header's
 * `buildConnectSrc()` reached no window at all. That is now true rather than
 * merely asserted, and what makes it true is `installSessionHooks` running on
 * the renderer's partition; `src/rendererSessionHooks.test.ts` pins it. Keep the
 * three files together — a string pin cannot tell you which policy is live.
 *
 * "Both policies" is the half that was first got wrong twice: the original fix
 * added the loopback `ws:` source to each, but gave only the header policy the
 * `wss:` an EXTERNAL backend's socket needs (`https:` does not cover `wss:` by
 * the very rule above), so an `https://` external daemon still hit the identical
 * failure with the meta tag alone blocking it. Any new source here belongs in
 * both files or in neither.
 *
 * WHAT THESE ASSERTIONS ARE, AND ARE NOT. They read source text; a CSP cannot be
 * exercised in jsdom, so this is a proxy for "the emitted policy admits the
 * socket". The fact itself was measured in real Chromium (Electron 39.8.10 /
 * Chromium 142) from a `file://` document — the packaged renderer's own origin —
 * against the four policies at issue:
 *   `…http://127.0.0.1:* https:`                 + `ws://127.0.0.1:…`  → BLOCKED
 *   `…http://127.0.0.1:* ws://127.0.0.1:* https:` + `ws://127.0.0.1:…`  → allowed
 *   `…http://127.0.0.1:* ws://127.0.0.1:* https:` + `wss://external/…`  → BLOCKED
 *   `… ws://127.0.0.1:* https: wss:`              + `wss://external/…`  → allowed
 * i.e. the shipped policy, verbatim, admits both sockets. The scheme rule that
 * decides all four is reimplemented below in `schemeMatches` and self-tested, so
 * these assertions fail on a policy that *reads* fine but does not reach.
 */

function resolveFromPackage(relative: string): string {
  const candidates = [
    join(process.cwd(), relative),
    join(process.cwd(), 'ui', 'desktop', relative),
  ];
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error(`could not locate ${relative} from ${process.cwd()}`);
  return found;
}

/** The `connect-src …;` run out of a full CSP string. */
function connectSrcOf(policy: string): string {
  const match = policy.match(/connect-src([^;]*)/);
  if (!match) throw new Error(`no connect-src directive in policy: ${policy.slice(0, 200)}`);
  return match[1].trim();
}

function metaCsp(): string {
  const html = readFileSync(resolveFromPackage('index.html'), 'utf-8');
  const match = html.match(
    /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/i
  );
  if (!match) throw new Error('index.html has no Content-Security-Policy meta tag');
  return match[1];
}

/**
 * The literal source list `main.ts`'s `buildConnectSrc()` starts from, as the
 * space-joined `connect-src` run it becomes — i.e. the JS string literals only,
 * with the `//` commentary between them dropped so a source expression quoted
 * inside a comment can never be mistaken for a real entry.
 */
function mainProcessConnectSources(): string {
  const source = readFileSync(resolveFromPackage('src/main.ts'), 'utf-8');
  const match = source.match(
    /const buildConnectSrc = \(\): string => \{\s*const sources = \[([^\]]*)\]/
  );
  if (!match) throw new Error('main.ts: could not find buildConnectSrc’s source list');
  // Whole-line comments only. A bare `//[^\n]*` would eat the `//` inside
  // `'http://127.0.0.1:*'` and silently shorten the list to nothing useful.
  const body = match[1].replace(/^[ \t]*\/\/.*$/gm, '');
  // Both quote styles, contents verbatim: `"'self'"` is double-quoted in the
  // source precisely BECAUSE the CSP keyword carries its own single quotes, and
  // dropping them would turn a keyword into an unparseable host source.
  const literals = [...body.matchAll(/"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'/g)].map(
    (m) => m[1] ?? m[2]
  );
  if (literals.length === 0) throw new Error('main.ts: buildConnectSrc’s source list parsed empty');
  return literals.join(' ');
}

/** `buildConnectSrc()`'s external-backend arm — the `sources.push` block. */
function mainProcessExternalArm(): string {
  const source = readFileSync(resolveFromPackage('src/main.ts'), 'utf-8');
  const match = source.match(
    /if \(settings\.externalBiorouterd\?\.enabled && settings\.externalBiorouterd\.url\) \{([\s\S]*?)\n {4}\}/
  );
  if (!match) throw new Error('main.ts: could not find buildConnectSrc’s external-backend arm');
  return match[1];
}

/**
 * CSP3 §6.6.2.6's scheme-part rule, the one thing this whole file exists
 * because we got wrong. A source expression's scheme covers a URL's scheme
 * only on this exact list of relaxations:
 *   http → https · ws → wss/http/https · wss → https
 * Notably ABSENT, and both are live bugs this file has caught:
 *   http ↛ ws   (the loopback socket, fixed in the commit that added this file)
 *   https ↛ wss (an external TLS backend's socket)
 */
function schemeMatches(exprScheme: string, urlScheme: string): boolean {
  if (exprScheme === urlScheme) return true;
  if (exprScheme === 'http') return urlScheme === 'https';
  if (exprScheme === 'ws')
    return urlScheme === 'wss' || urlScheme === 'http' || urlScheme === 'https';
  if (exprScheme === 'wss') return urlScheme === 'https';
  return false;
}

/**
 * Does one `connect-src` source expression permit `url`?
 *
 * Deliberately partial: it models the source forms these two policies actually
 * use (`scheme:`, `scheme://host`, `scheme://host:port`, `scheme://host:*`) and
 * THROWS on anything else rather than quietly answering `false` — an
 * unrecognised source silently reading as "does not match" is how a matcher
 * like this rots into a test that passes for the wrong reason. Quoted keywords
 * (`'self'`) are treated as no-match: `'self'` is the renderer's own origin
 * (a vite dev-server URL, or `file://` when packaged) and never covers the
 * daemon socket, so nothing here may depend on it.
 */
function sourceAllows(source: string, url: URL): boolean {
  if (source.startsWith("'")) return false;
  const urlScheme = url.protocol.replace(/:$/, '');

  const schemeOnly = /^([a-z][a-z0-9+.-]*):$/i.exec(source);
  if (schemeOnly) return schemeMatches(schemeOnly[1].toLowerCase(), urlScheme);

  const hostSource = /^([a-z][a-z0-9+.-]*):\/\/([^/:]+)(?::(\*|\d+))?$/i.exec(source);
  if (!hostSource) throw new Error(`unmodelled CSP source expression: ${source}`);
  const [, scheme, host, port] = hostSource;
  if (!schemeMatches(scheme.toLowerCase(), urlScheme)) return false;
  if (host.toLowerCase() !== url.hostname.toLowerCase()) return false;
  if (port === undefined || port === '*') return true;
  return port === url.port;
}

/** Does a whole `connect-src` source list permit `url`? */
function policyAllows(connectSrc: string, url: string): boolean {
  const parsed = new URL(url);
  return connectSrc
    .split(/\s+/)
    .filter(Boolean)
    .some((source) => sourceAllows(source, parsed));
}

describe('workspace channel CSP', () => {
  it('the index.html meta policy allows a ws: connection to the loopback daemon', () => {
    const connectSrc = connectSrcOf(metaCsp());
    expect(policyAllows(connectSrc, 'ws://127.0.0.1:62736/ui/workspace')).toBe(true);
  });

  it('the main-process header policy allows a ws: connection to the loopback daemon', () => {
    expect(policyAllows(mainProcessConnectSources(), 'ws://127.0.0.1:62736/ui/workspace')).toBe(
      true
    );
  });

  /**
   * The same defect, one config away. `buildConnectSrc()` pushes an external
   * backend's origin AND its ws form, but the meta tag's blanket `https:` does
   * not cover `wss:` (see `schemeMatches`) — so pointing Settings → Biorouter
   * Server at an `https://` daemon (a supported, validated input:
   * `ExternalBackendSection.tsx` accepts exactly `http:` and `https:`) left the
   * header policy permitting the socket and the meta policy still blocking it,
   * reproducing the identical "GUI half silently inert, `gui_attached: false`
   * with the app on screen" failure Task 31 was created to catch.
   *
   * Measured, not reasoned: in Electron 39.8.10 / Chromium 142, a `file://`
   * document under `connect-src 'self' http://127.0.0.1:* ws://127.0.0.1:*
   * https:` fires `securitypolicyviolation` with
   * `blockedURI: wss://external.example.invalid/ui/workspace`; adding `wss:`
   * lets it through.
   */
  it('the index.html meta policy allows the wss: socket of an external TLS backend', () => {
    expect(policyAllows(connectSrcOf(metaCsp()), 'wss://backend.example.org/ui/workspace')).toBe(
      true
    );
  });

  /**
   * Regression guard, not a defect test — this arm has been correct since the
   * commit that introduced it. It pins the coupling that makes it correct: the
   * CSP entry must be derived the way the hook derives the socket URL, so
   * changing one and not the other is a test failure rather than a dead socket.
   */
  it('the external-backend arm allows both the origin and the ws form the hook derives', () => {
    const arm = mainProcessExternalArm();
    expect(arm).toMatch(/sources\.push\(externalUrl\.origin\)/);
    expect(arm).toMatch(/sources\.push\(externalUrl\.origin\.replace\(\/\^http\/, 'ws'\)\)/);

    const hook = readFileSync(resolveFromPackage('src/hooks/useWorkspaceChannel.ts'), 'utf-8');
    expect(hook).toContain(".replace(/^http/, 'ws')");
  });

  it('does not settle for scheme sources CSP will not stretch to the socket', () => {
    // The two ways someone "fixes" a blocked socket without naming its scheme:
    // widen to a wildcard, or lean on a scheme source that does not reach it.
    // (`'unsafe-eval'` used to be asserted here. It is not a `connect-src`
    // keyword at all, so its absence was trivially true — a vacuous assertion
    // dressed as a guard.)
    for (const connectSrc of [connectSrcOf(metaCsp()), mainProcessConnectSources()]) {
      expect(connectSrc.split(/\s+/)).not.toContain('*');
      expect(policyAllows(connectSrc, 'ws://127.0.0.1:62736/ui/workspace')).toBe(true);
    }
    // Cleartext WebSockets stay pinned to loopback, mirroring the meta policy's
    // own cleartext http allowance (`http://127.0.0.1:*`, not blanket `http:`).
    expect(connectSrcOf(metaCsp()).split(/\s+/)).not.toContain('ws:');
    expect(policyAllows(connectSrcOf(metaCsp()), 'ws://evil.example/ui/workspace')).toBe(false);
  });

  it('the scheme matcher these assertions rest on is not vacuous', () => {
    // Without this, a matcher that answered `false` for everything would make
    // the `.toBe(false)` cases above pass and the `.toBe(true)` cases the only
    // real content. Both pre-fix policies are reproduced verbatim.
    const beforeLoopbackFix = "'self' http://127.0.0.1:* https:";
    expect(policyAllows(beforeLoopbackFix, 'ws://127.0.0.1:62736/ui/workspace')).toBe(false);

    const beforeExternalFix = "'self' http://127.0.0.1:* ws://127.0.0.1:* https:";
    expect(policyAllows(beforeExternalFix, 'wss://backend.example.org/ui/workspace')).toBe(false);
    // …and the relaxations CSP *does* grant still work, so it is not merely strict.
    expect(policyAllows(beforeExternalFix, 'https://backend.example.org/x')).toBe(true);
    expect(policyAllows('ws://127.0.0.1:*', 'wss://127.0.0.1:62736/x')).toBe(true);
    expect(() => policyAllows('data:', 'ws://127.0.0.1:1/x')).not.toThrow();
    expect(() => policyAllows('*.example.org', 'ws://127.0.0.1:1/x')).toThrow(/unmodelled/);
  });
});
