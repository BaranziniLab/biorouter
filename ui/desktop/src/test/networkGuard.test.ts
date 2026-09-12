/**
 * The regression guard for the ambient-network defect.
 *
 * The suite passed with `ollama serve` running and passed without it, while
 * `ProviderCatalog.test.tsx` took a different path through `OllamaInlineCard` in
 * each case (see src/test/networkGuard.ts). Nothing could have noticed, because
 * nothing was looking at whether a request had been attempted. These tests are
 * what looks.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import {
  KNOWN_NETWORK_ATTEMPTS,
  NEVER_ALLOWED_HOSTS,
  OFFLINE_FETCH_MESSAGE,
  assertNoUnexpectedNetworkAttempts,
  recordedAttempts,
  resetRecordedAttempts,
  specKey,
  unexpectedAttempts,
} from './networkGuard';

const DESKTOP_ROOT = join(__dirname, '../..');

// This spec deliberately triggers the offline fetch, so it must leave nothing
// recorded for `setup.ts`'s own check to trip over. Registered here, which vitest
// runs BEFORE the hook in setup.ts.
afterEach(() => {
  resetRecordedAttempts();
});

describe('the offline fetch installed for every spec', () => {
  it('rejects instead of leaving the machine, and names the URL', async () => {
    await expect(fetch('http://127.0.0.1:11434/api/tags')).rejects.toThrow(
      /Un-stubbed network request in a test: GET http:\/\/127\.0\.0\.1:11434\/api\/tags/
    );
  });

  it('records the attempt even though the caller may swallow the rejection', async () => {
    // This is the shape that made the defect invisible: `ollamaDetection.ts`
    // catches the failure and returns `{ isRunning: false }`, so a rejecting stub
    // on its own reports nothing. The record is what survives the catch.
    await fetch('http://localhost/some/route').catch(() => undefined);
    expect(recordedAttempts()).toContain('http://localhost/some/route');
  });

  it('reads a Request and a URL object, not only a string', async () => {
    await fetch(new URL('http://localhost/from-url-object')).catch(() => undefined);
    await fetch(new Request('http://localhost/from-request')).catch(() => undefined);
    expect(recordedAttempts()).toEqual(
      expect.arrayContaining(['http://localhost/from-url-object', 'http://localhost/from-request'])
    );
  });

  it('fails a spec that has no allowance, naming the spec and the URL', async () => {
    const invented = '/repo/ui/desktop/src/components/Invented.test.tsx';
    await fetch('http://127.0.0.1:11434/api/tags').catch(() => undefined);
    expect(() => assertNoUnexpectedNetworkAttempts(invented)).toThrow(
      /src\/components\/Invented\.test\.tsx/
    );
    // Drained by the report, so the same attempt is not blamed on the next test.
    expect(() => assertNoUnexpectedNetworkAttempts(invented)).not.toThrow();
  });

  it('passes an attempt the spec has an allowance for', () => {
    const spec = 'src/components/settings/SettingsView.test.tsx';
    expect(unexpectedAttempts(spec, ['http://localhost/privacy/disclosure'])).toEqual([]);
    expect(unexpectedAttempts(spec, ['http://localhost/sessions'])).toEqual([
      'http://localhost/sessions',
    ]);
  });

  it('fails closed on a path shape it does not recognise', () => {
    // An unknown key must not accidentally match an entry, so no allowance
    // applies and the attempt is reported.
    expect(specKey(undefined)).toBe('<unknown spec>');
    expect(unexpectedAttempts(undefined, ['http://localhost/anything'])).toEqual([
      'http://localhost/anything',
    ]);
  });
});

describe('the allowance table', () => {
  it('never grants the Ollama host to any spec', () => {
    // The one host a developer is genuinely likely to be running, and the one
    // this whole guard exists for. An entry here would re-open the defect while
    // every test still passed.
    const offenders = Object.entries(KNOWN_NETWORK_ATTEMPTS).flatMap(([spec, urls]) =>
      urls
        .filter((url) => NEVER_ALLOWED_HOSTS.some((host) => url.includes(host)))
        .map((url) => `${spec} -> ${url}`)
    );
    expect(offenders).toEqual([]);
  });

  it('does not grant ProviderCatalog.test.tsx the reach it used to have', () => {
    // Named explicitly because this is the spec the defect was found in: it
    // mocks `utils/ollamaDetection` now, so it attempts nothing on 11434 and
    // needs no allowance for it.
    const allowed =
      KNOWN_NETWORK_ATTEMPTS['src/components/settings/providers/ProviderCatalog.test.tsx'];
    expect(allowed).toBeDefined();
    expect(allowed!.some((url) => url.includes('11434'))).toBe(false);
  });

  it('names only spec files that exist, so the table cannot rot', () => {
    const missing = Object.keys(KNOWN_NETWORK_ATTEMPTS).filter(
      (spec) => !existsSync(join(DESKTOP_ROOT, spec))
    );
    expect(missing).toEqual([]);
  });

  it('keeps the message prefix the two reports share', () => {
    // Both the rejection and the end-of-test report open with it, so a developer
    // greps one string.
    expect(OFFLINE_FETCH_MESSAGE).toBe('Un-stubbed network request in a test:');
  });
});
