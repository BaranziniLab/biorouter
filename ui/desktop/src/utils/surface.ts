/**
 * Which shell is this renderer running inside — the desktop application, or an
 * ordinary web browser pointed at `biorouter serve`?
 *
 * The two surfaces are not equally capable, and the difference is deliberate.
 * `docs/deployment/serve-decisions.md` **SD-1** rules that a browser session
 * cannot change its model or provider: the daemon a `serve` session talks to is
 * started with no proof-of-user mechanism (SD-7), so every write to a
 * capability config key — `POST /config/set_provider`, and `/config/upsert` or
 * `/config/remove` on `BIOROUTER_PROVIDER` and its four siblings — is refused
 * with a 409 by the privacy boundary in issue #56 (DR-16).
 *
 * That refusal is the feature, not a bug to route around. What SD-1 also
 * requires is that *"the interface must explain the refusal rather than appear
 * broken. A disabled picker with a reason is the requirement; a 409 toast is
 * not."* — the refusal body is addressed to an AI agent and tells the reader to
 * go and use the desktop application, which is no help at all to a human in a
 * browser. This module is how a control finds out that it should explain
 * itself **before** the user reaches the refusal.
 *
 * ⚠ **Detection reads the DOM, not a module-level snapshot.** `renderer.tsx`
 * stamps the marker below onto `<html>` at the moment it installs the browser
 * shim, which is before React mounts but *after* this module may have been
 * evaluated. A value captured at import time would therefore be read too early
 * and would answer `desktop` on the very surface this exists to detect.
 *
 * ⚠ **The marker is on `<html>`, not on `<body>`.** `renderer.tsx` sets both,
 * and the body class is what stylesheets hook; the `documentElement` dataset is
 * what code should ask, because it survives anything that replaces the body.
 */

/** The shell the renderer is running inside. */
export type AppSurface = 'desktop' | 'browser';

/**
 * The value `renderer.tsx` writes to `document.documentElement.dataset.biorouterSurface`.
 *
 * Historical spelling: the browser-served build was called "headless" before
 * `biorouter serve` existed (SD-5 kept `headless` as a command alias for the
 * same reason). The attribute value is left alone so a page served by an older
 * daemon still identifies itself; the *type* above uses the current word.
 */
export const BROWSER_SURFACE_MARKER = 'headless';

/** The class `renderer.tsx` adds to `<body>` on the same surface. */
export const BROWSER_SURFACE_BODY_CLASS = 'biorouter-headless-browser';

/** Which shell this renderer is running inside, asked fresh every call. */
export function currentSurface(): AppSurface {
  if (typeof document === 'undefined') return 'desktop';
  return document.documentElement?.dataset?.biorouterSurface === BROWSER_SURFACE_MARKER
    ? 'browser'
    : 'desktop';
}

/**
 * Is Biorouter being served to a web browser?
 *
 * The question every provider/model control should ask before offering itself.
 * A `true` here means the daemon will refuse the write (SD-1), so the control
 * must be disabled with a reason rather than left to fail.
 */
export function isBrowserSurface(): boolean {
  return currentSurface() === 'browser';
}

/**
 * The config keys a browser session may not write, mirroring
 * `CAPABILITY_CONFIG_KEYS` in `crates/biorouter/src/privacy/config_keys.rs`.
 *
 * ⚠ **This list is a mirror and can drift.** It exists because the generic
 * config editor (`settings/config/ConfigSettings.tsx`) renders every non-secret
 * key as an editable field, and only these six 409 — so a blanket disable
 * there would be wrong about most of the page. Nothing enforces the agreement:
 * if a key is added to the Rust constant it must be added here too, and the
 * cost of missing one is a Save button that still produces the agent-facing
 * 409 this whole change exists to replace. Every *other* surface touched by
 * this change asks {@link isBrowserSurface} instead, because each of them
 * writes `BIOROUTER_PROVIDER` by construction and has no key to test.
 */
export const HOST_MANAGED_CONFIG_KEYS: readonly string[] = [
  'BIOROUTER_PROVIDER',
  'BIOROUTER_LEAD_MODEL',
  'BIOROUTER_LEAD_PROVIDER',
  'OLLAMA_HOST',
  'LLAMACPP_EXTERNAL_HOST',
  // A capability key since 2026-10: `ollama`'s `tier()` reads the model name
  // (a `-cloud` tag is Public through a loopback host), so `/config/upsert`
  // refuses it on a browser surface exactly as it refuses `OLLAMA_HOST`.
  'BIOROUTER_MODEL',
];

/** Would writing `key` over HTTP be refused on a browser-served surface? */
export function isHostManagedConfigKey(key: string): boolean {
  return HOST_MANAGED_CONFIG_KEYS.includes(key);
}
