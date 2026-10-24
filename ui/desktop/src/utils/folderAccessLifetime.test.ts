import { afterEach, describe, expect, it, vi } from 'vitest';
import { folderAccessLifetime } from './folderAccessLifetime';
import { BROWSER_SURFACE_MARKER } from './surface';

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  vi.unstubAllGlobals();
});

describe('folder session lifetime notice', () => {
  it.each([
    { surface: 'desktop', local: true, desktopLifetime: true },
    { surface: 'desktop', local: false, desktopLifetime: false },
    { surface: 'desktop', local: undefined, desktopLifetime: false },
    { surface: 'browser', local: true, desktopLifetime: false },
  ])('$surface local=$local', ({ surface, local, desktopLifetime }) => {
    document.documentElement.dataset.biorouterSurface =
      surface === 'browser' ? BROWSER_SURFACE_MARKER : '';
    vi.stubGlobal('window', { appConfig: { get: () => local } });
    const text = folderAccessLifetime();
    expect(text.includes('desktop app quits or restarts')).toBe(desktopLifetime);
    expect(text.includes('backend restarts')).toBe(!desktopLifetime);
    expect(text.includes('does not revoke them')).toBe(!desktopLifetime);
  });

  it('does not promise desktop expiry when older preload config is unavailable', () => {
    vi.stubGlobal('window', {});
    expect(folderAccessLifetime()).toContain('backend restarts');
  });
});
