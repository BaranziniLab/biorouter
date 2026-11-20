/**
 * The user agent the Biorouter Browser presents (BB-07).
 *
 * Electron's default user agent carries `Electron/…` and the app's own name, and some sites treat
 * that as an embedded app: they refuse sign in, or serve a reduced page. The browser presents the
 * plain Chrome string for the bundled Chromium instead, in Chrome's reduced form (major version
 * only, `0.0.0` for the rest, and a frozen platform token), which is what Chrome itself sends.
 *
 * Measured on Electron 39.8.10: after `session.setUserAgent` the request header is this string,
 * `navigator.userAgentData.brands` lists only `Chromium` and `Not_A Brand`, and no client hint
 * names Electron, so no CDP override is needed for the brands.
 */
export function chromeUserAgent(platform: string, chromeVersion: string): string {
  const major = /^\d+/.exec(chromeVersion)?.[0] ?? '142';
  const platformToken =
    platform === 'darwin'
      ? 'Macintosh; Intel Mac OS X 10_15_7'
      : platform === 'win32'
        ? 'Windows NT 10.0; Win64; x64'
        : 'X11; Linux x86_64';
  return `Mozilla/5.0 (${platformToken}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36`;
}

/** `Accept-Language` from the person's preferred languages, most preferred first. */
export function acceptLanguages(preferred: readonly string[]): string {
  const cleaned = preferred
    .map((language) => language.trim())
    .filter((language) => /^[a-z]{2,3}(?:-[a-z0-9]{2,8})*$/i.test(language));
  const unique = [...new Set(cleaned)].slice(0, 8);
  return unique.length > 0 ? unique.join(',') : 'en-US,en';
}
