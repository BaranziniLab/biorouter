import { afterEach, describe, expect, it } from 'vitest';
import {
  BROWSER_SURFACE_BODY_CLASS,
  BROWSER_SURFACE_MARKER,
  HOST_MANAGED_CONFIG_KEYS,
  currentSurface,
  isBrowserSurface,
  isHostManagedConfigKey,
} from './surface';

/**
 * The capability helper behind SD-1's "disabled picker with a reason".
 *
 * ⚠ Every test in this file fails against the pre-change tree for the same
 * trivial reason — `utils/surface.ts` did not exist, and each provider/model
 * control asked nothing at all before offering itself. What the individual
 * cases below are really for is the *next* change: they pin the three
 * properties a plausible rewrite of this module gets wrong.
 */

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('currentSurface', () => {
  it('is the desktop when nothing marked the page', () => {
    expect(currentSurface()).toBe('desktop');
    expect(isBrowserSurface()).toBe(false);
  });

  it('is the browser once renderer.tsx has stamped the marker', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    expect(currentSurface()).toBe('browser');
    expect(isBrowserSurface()).toBe(true);
  });

  /**
   * ⚠ **The regression this file exists to catch.**
   *
   * `renderer.tsx` stamps the marker at the moment it installs the browser
   * shim, which is *after* this module may already have been evaluated —
   * modules are hoisted, the stamp is a statement. A `const isBrowser =
   * document…` captured at import time would therefore sample the DOM before
   * anything wrote to it and answer `desktop` forever, on the one surface the
   * helper exists to detect. Every control would then quietly go back to
   * offering itself and refusing with a 409.
   *
   * This test asserts the answer changes *after* import, which a memoised
   * implementation cannot do.
   */
  it('re-reads the DOM rather than capturing a value at import time', () => {
    expect(currentSurface()).toBe('desktop');
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    expect(currentSurface()).toBe('browser');
    delete document.documentElement.dataset.biorouterSurface;
    expect(currentSurface()).toBe('desktop');
  });

  /**
   * A marker nobody recognises is not evidence of a browser. Failing towards
   * `desktop` here fails towards *offering* the picker, which is the wrong
   * direction — but the alternative, treating any value as browser-served,
   * would let an unrelated future `data-biorouter-surface` disable the model
   * picker in the desktop application.
   */
  it('treats an unrecognised marker as the desktop', () => {
    document.documentElement.dataset.biorouterSurface = 'something-else';
    expect(currentSurface()).toBe('desktop');
  });

  it('agrees with the class renderer.tsx puts on the body', () => {
    expect(BROWSER_SURFACE_BODY_CLASS).toBe('biorouter-headless-browser');
    expect(BROWSER_SURFACE_MARKER).toBe('headless');
  });
});

describe('isHostManagedConfigKey', () => {
  /**
   * ⚠ **A mirror of `CAPABILITY_CONFIG_KEYS` in
   * `crates/biorouter/src/privacy/config_keys.rs`, and it must stay exactly
   * that size.** The generic config editor renders every non-secret key; only
   * these six 409 on a browser surface. A blanket "disable the page" would
   * pass a test that only checked the six, so the negative cases below are the
   * load-bearing half. `BIOROUTER_MODEL` is in the Rust list since 2026-10,
   * because `ollama`'s `tier()` reads the model name; the lead model and the
   * provider were already there.
   */
  it('names the six keys the daemon refuses, and no others', () => {
    expect([...HOST_MANAGED_CONFIG_KEYS].sort()).toEqual([
      'BIOROUTER_LEAD_MODEL',
      'BIOROUTER_LEAD_PROVIDER',
      'BIOROUTER_MODEL',
      'BIOROUTER_PROVIDER',
      'LLAMACPP_EXTERNAL_HOST',
      'OLLAMA_HOST',
    ]);
  });

  it('does not claim ordinary settings are fixed by the host', () => {
    expect(isHostManagedConfigKey('BIOROUTER_PROVIDER')).toBe(true);
    expect(isHostManagedConfigKey('OLLAMA_HOST')).toBe(true);
    // A capability key: `ollama`'s `tier()` reads the model name (a `-cloud`
    // tag is Public even through a loopback OLLAMA_HOST), so `/config/upsert`
    // refuses it on a browser surface. Until 2026-10 it was let through.
    expect(isHostManagedConfigKey('BIOROUTER_MODEL')).toBe(true);
    expect(isHostManagedConfigKey('OLLAMA_TIMEOUT')).toBe(false);
    expect(isHostManagedConfigKey('BIOROUTER_CONTEXT_LIMIT')).toBe(false);
  });
});
