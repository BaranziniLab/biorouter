import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getExtensionScrollBehavior } from './ExtensionsView';

const mocks = vi.hoisted(() => ({
  addExtension: vi.fn(async () => undefined),
  removeExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  read: vi.fn(async () => null),
  getProviders: vi.fn(async () => []),
}));

// `useConfig` THROWS outside a ConfigProvider and the context object itself is
// module-private, so it cannot be wrapped — the component must be given a
// mocked hook. Same shape as `settings/extensions/ExtensionsSection.test.tsx`,
// including `usePrivacyTiersEnabled`, which every extension card's
// `PrivacyBadge` reads off this same context and which throws rather than
// defaulting when a mock omits it.
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: [],
    addExtension: mocks.addExtension,
    removeExtension: mocks.removeExtension,
    getExtensions: mocks.getExtensions,
    read: mocks.read,
    getProviders: mocks.getProviders,
  }),
  usePrivacyTiersEnabled: () => true,
}));

// `settings/extensions/index` re-exports the real extension-manager calls,
// which would hit the daemon.
vi.mock('../settings/extensions', () => ({
  toggleExtensionDefault: vi.fn(async () => undefined),
  activateExtensionDefault: vi.fn(async () => undefined),
  deleteExtension: vi.fn(async () => undefined),
}));

import ExtensionsView from './ExtensionsView';

describe('getExtensionScrollBehavior', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    Reflect.deleteProperty(window, 'matchMedia');
  });

  it('uses instant scrolling when reduced motion is requested', () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: true }) as never),
    });

    expect(getExtensionScrollBehavior()).toBe('auto');
  });

  it('uses smooth scrolling otherwise', () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: false }) as never),
    });

    expect(getExtensionScrollBehavior()).toBe('smooth');
  });
});

/**
 * The header this view used to author for itself, now `Layout/PageHeader`.
 *
 * ⚠ Asserted through the DOM rather than by grepping the source, for the reason
 * `schedule/SchedulesView.test.tsx` records: a source grep for `<PageHeader`
 * passes on a view that mounts the shared header AND has quietly grown a second
 * title or a stray action beside it. What jsdom cannot see — that
 * `.biorouter-settings-control-strip` and `size="chat"` resolve to anything at
 * all — belongs in `styles/measures.test.ts`, which asserts those at the source
 * precisely because Tailwind never runs here.
 */
describe('ExtensionsView — the shared page header', () => {
  // `SearchView` subscribes to four Find-menu channels on mount and calls the
  // disposers it gets back, so `window.electron.on` must return a function
  // rather than `undefined`. `platform` is set rather than left absent so the
  // shortcut assertion below names a branch on purpose instead of relying on
  // jsdom happening to have no `window.electron` at all.
  beforeEach(() => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      value: { platform: 'linux', on: vi.fn(() => vi.fn()) },
    });
  });

  afterEach(() => {
    Reflect.deleteProperty(window, 'electron');
  });

  const renderView = () =>
    render(<ExtensionsView onClose={vi.fn()} setView={vi.fn()} viewOptions={{}} />);

  it('has exactly one page title, and no control on its row', async () => {
    renderView();

    const heading = await screen.findByRole('heading', { level: 1, name: 'Extensions' });
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    // The operator's decision, 2026-09-07: actions sit on their own line under
    // the description, so the title's row holds the title (and, in views that
    // have one, an adornment) and nothing clickable.
    expect((heading.parentElement as HTMLElement).querySelector('button')).toBeNull();
  });

  it('keeps the search-shortcut sentence in the description', async () => {
    renderView();

    // `getSearchShortcutText()` is `Ctrl+F` off a Mac and `⌘F` on one, and
    // jsdom sets no `window.electron`, so the non-Mac branch is what renders.
    // The sentence is load-bearing: it is the only place the view tells the
    // user the shortcut exists.
    const description = await screen.findByText(/to search\./);
    expect(description.textContent).toContain('Ctrl+F to search.');
    expect(description.textContent).toContain('apply to all new chats');
  });

  it('puts all three actions in one control strip under the description', async () => {
    renderView();

    const add = await screen.findByRole('button', { name: 'Add extension' });
    const strip = add.closest('.biorouter-settings-control-strip');
    expect(strip).not.toBeNull();
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Browse extensions' }));
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Add custom extension' }));
  });

  /**
   * V7. The three buttons carried `className="flex items-center gap-2"`, and a
   * bare `flex` FLIPS `buttonVariants`' own `inline-flex` through
   * tailwind-merge — the defect that rendered a row action elsewhere as a
   * full-width bar. jsdom runs no Tailwind, so the class STRING is the only
   * observable: assert the button never carries it rather than measuring a box
   * that has no layout here.
   */
  it('lets the Button primitive own its own layout', async () => {
    renderView();

    for (const name of ['Add extension', 'Browse extensions', 'Add custom extension']) {
      const button = await screen.findByRole('button', { name });
      expect(button.className.split(/\s+/)).not.toContain('flex');
    }
  });
});
