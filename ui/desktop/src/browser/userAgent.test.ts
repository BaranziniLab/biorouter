import { describe, expect, it } from 'vitest';
import { acceptLanguages, chromeUserAgent } from './userAgent';
import { browserTestHooksEnabled } from './testHooks';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

describe('the user agent sites see (BB-07)', () => {
  it.each([
    ['darwin', 'Macintosh; Intel Mac OS X 10_15_7'],
    ['win32', 'Windows NT 10.0; Win64; x64'],
    ['linux', 'X11; Linux x86_64'],
  ])('is plain reduced Chrome on %s', (platform, token) => {
    const ua = chromeUserAgent(platform, '142.0.7444.265');
    expect(ua).toBe(
      `Mozilla/5.0 (${token}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36`
    );
    expect(ua).not.toMatch(/Electron|Biorouter/i);
  });

  it('builds Accept-Language from valid language tags only', () => {
    expect(acceptLanguages(['en-US', 'fr', 'en-US', 'not a tag', ''])).toBe('en-US,fr');
    expect(acceptLanguages([])).toBe('en-US,en');
  });
});

describe('the browser test hooks', () => {
  it('need an unpackaged app AND BIOROUTER_E2E=1', () => {
    expect(browserTestHooksEnabled({ BIOROUTER_E2E: '1' }, false)).toBe(true);
    expect(browserTestHooksEnabled({ BIOROUTER_E2E: '1' }, true)).toBe(false);
    expect(browserTestHooksEnabled({}, false)).toBe(false);
    expect(browserTestHooksEnabled({ BIOROUTER_E2E: 'true' }, false)).toBe(false);
  });

  it('are wired in main through that one gate', () => {
    const main = readFileSync(join(__dirname, '..', 'main.ts'), 'utf8');
    expect(main).toContain('browserTestHooksEnabled(process.env, app.isPackaged)');
    expect(main).toContain(
      'configureBrowserTestHosts(process.env.BIOROUTER_BROWSER_TEST_HOSTS, testHooks)'
    );
    expect(main).toContain(
      'testHooks ? (process.env.BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR ?? null) : null'
    );
    // Nothing else reads the variables.
    expect(main.match(/BIOROUTER_BROWSER_TEST_HOSTS/g)).toHaveLength(1);
    expect(main.match(/BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR/g)).toHaveLength(1);
  });
});
