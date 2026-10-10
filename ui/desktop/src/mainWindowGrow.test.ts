import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Spec 3.16, Windows: when the app grows its own window (opening a preview in a
 * window too narrow for it, `window:ensure-content-width`), the OS animates the
 * grow unless the person turned on Reduce Motion, and the app never shrinks the
 * window. The main process cannot be mounted in a unit test, so the handler is
 * read at the source.
 */
const MAIN = readFileSync(join(__dirname, 'main.ts'), 'utf8');

function handlerBody(): string {
  const start = MAIN.indexOf("ipcMain.handle('window:ensure-content-width'");
  expect(start, 'no window:ensure-content-width handler').toBeGreaterThan(-1);
  const end = MAIN.indexOf('\n});\n', start);
  return MAIN.slice(start, end);
}

describe('the app-initiated window grow', () => {
  it('asks the system for Reduce Motion', () => {
    expect(MAIN).toMatch(
      /function windowResizeMayAnimate\(\): boolean \{[\s\S]*?systemPreferences\.getAnimationSettings\(\)\.prefersReducedMotion/
    );
  });

  it('passes that answer to both resize calls, never a literal true', () => {
    const body = handlerBody();
    expect(body).toMatch(/const animate = windowResizeMayAnimate\(\);/);
    expect(body).toMatch(/win\.setContentSize\([^)]*,\s*animate\)/);
    expect(body).toMatch(/win\.setBounds\([\s\S]*?,\s*animate\)/);
    expect(body).not.toMatch(/,\s*true\)/);
  });

  it('only ever grows: a wide enough window is left alone', () => {
    expect(handlerBody()).toMatch(
      /if \(currentContentBounds\.width >= minWidth[\s\S]*?\) \{\s*return \{\s*expanded: false/
    );
  });
});
