/**
 * What a marketplace download answered, read one way on every surface.
 *
 * ⚠ **`'error' in result` is not a failure test.** The desktop app's
 * `registry:download` handler answers `{ path }` or `{ error }`, and both
 * Browse modals tested `'error' in dl` — correct for that shape and nothing
 * else. `biorouter serve` answers from `POST /headless/registry/download`,
 * whose Rust response carries BOTH keys with one of them `null`
 * (`{"path":"…/primer-design.zip","error":null}`), and the browser shim handed
 * that body straight through as if it matched the declared type. So the key
 * was always present, every download counted as failed, and nothing reached
 * `/skills/packages/install`: in a browser every Browse skills install ended
 * in "2 selections were not installed | Alignment: failed", with no reason,
 * because the "error" was `null`.
 *
 * The answer here is decided by VALUES: a non-empty `error` string is a
 * failure, a non-empty `path` string is a success, and anything else — a
 * `null` body from an unreachable daemon, a shim's `{ success: false }` — is a
 * failure carrying `fallback`, so a caller always has a sentence to show.
 */

/** The declared shape of `window.electron.downloadRegistryAsset`'s answer. */
export type RegistryDownloadResult = { path: string } | { error: string };

export function readRegistryDownload(raw: unknown, fallback: string): RegistryDownloadResult {
  if (raw && typeof raw === 'object') {
    const { path, error } = raw as { path?: unknown; error?: unknown };
    if (typeof error === 'string' && error.trim()) return { error };
    if (typeof path === 'string' && path) return { path };
  }
  return { error: fallback };
}
