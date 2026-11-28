import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * `img-src` in the two policies that govern the renderer document (OP-P0-09,
 * the owner's answer to D15).
 *
 * Remote images in markdown load on a person's click. In the desktop app the
 * click goes to the main process, which fetches that one image with no cookie,
 * referrer or cache (`utils/remoteImageFetch.ts`) and hands back its bytes, so
 * the renderer shows it from a `blob:` URL. That is what lets the desktop
 * header drop `https:`: no `<img>` in the desktop renderer can reach the web on
 * its own, so a renderer that forgot the click would still request nothing.
 *
 * The `<meta>` in index.html keeps `https:` for one reason: it is also the only
 * image policy of the page `biorouter serve` sends to a browser, where there is
 * no main process to fetch through and the click sets the address itself. In
 * the desktop both policies apply and the stricter wins, so the meta's
 * `https:` reaches no desktop window.
 *
 * WHAT THESE ASSERTIONS ARE, AND ARE NOT. They read source text; jsdom has no
 * CSP engine. They fail when someone widens the desktop header again, or drops
 * the click path's main-process half while the header still needs it.
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

function directiveOf(policy: string, directive: string): string[] {
  const match = policy.match(new RegExp(`${directive}([^;]*)`));
  if (!match) throw new Error(`no ${directive} directive in policy: ${policy.slice(0, 200)}`);
  return match[1].trim().split(/\s+/).filter(Boolean);
}

function mainSource(): string {
  return readFileSync(resolveFromPackage('src/main.ts'), 'utf-8').replace(/^[ \t]*\/\/.*$/gm, '');
}

/** The sole `img-src …` literal `main.ts` concatenates into the renderer's header. */
function headerImgSrc(): string[] {
  const literals = [...mainSource().matchAll(/"(img-src[^"]*)"/g)].map((m) => m[1]);
  if (literals.length !== 1) {
    throw new Error(`expected exactly one img-src literal in main.ts, found ${literals.length}`);
  }
  return directiveOf(literals[0], 'img-src');
}

function metaImgSrc(): string[] {
  const html = readFileSync(resolveFromPackage('index.html'), 'utf-8');
  const match = html.match(
    /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/i
  );
  if (!match) throw new Error('index.html has no Content-Security-Policy meta tag');
  return directiveOf(match[1], 'img-src');
}

describe('img-src', () => {
  it('admits no web image in the desktop header, only the app and what it makes', () => {
    expect(headerImgSrc().sort()).toEqual(["'self'", 'blob:', 'data:'].sort());
  });

  it('keeps the app, data and blob images in the shared meta policy', () => {
    const meta = metaImgSrc();
    for (const source of ["'self'", 'data:', 'blob:']) expect(meta).toContain(source);
    expect(meta).not.toContain('http:');
    expect(meta).not.toContain('*');
  });

  it('is not contradicted: the desktop has a main-process path for a clicked image', () => {
    const main = mainSource();
    expect(main).toMatch(/ipcMain\.handle\(\s*REMOTE_IMAGE_CHANNEL/);
    expect(main).toMatch(/fetchRemoteImage\(/);
    const preload = readFileSync(resolveFromPackage('src/preload.ts'), 'utf-8');
    expect(preload).toMatch(
      /loadRemoteImage:\s*\(url: string\)\s*=>\s*ipcRenderer\.invoke\(\s*REMOTE_IMAGE_CHANNEL/
    );
  });
});
