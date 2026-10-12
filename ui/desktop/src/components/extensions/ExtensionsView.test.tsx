import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getExtensionScrollBehavior } from './ExtensionsView';
import { EXTENSIONS_COPY } from './copy';
import { isMacPlatform, uiCopy } from '../ui/copy';

const mocks = vi.hoisted(() => ({
  addExtension: vi.fn(async () => undefined),
  removeExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  read: vi.fn(async () => null),
  getProviders: vi.fn(async () => []),
  extensionsList: [] as Array<Record<string, unknown>>,
}));

// `useConfig` THROWS outside a ConfigProvider and the context object itself is
// module-private, so it cannot be wrapped — the component must be given a
// mocked hook. Same shape as `settings/extensions/ExtensionsSection.test.tsx`,
// including `usePrivacyTiersEnabled`, which every extension card's
// `PrivacyBadge` reads off this same context and which throws rather than
// defaulting when a mock omits it.
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: mocks.extensionsList,
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

// Radix's ScrollArea needs a ResizeObserver jsdom does not have, and the view only needs the
// children (the same stand-in the Skills, Workflows and Scheduler tests use).
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="list-scroller">{children}</div>
  ),
}));
// The Browse modal fetches the marketplace; its own suite covers it.
vi.mock('../baam/BrowseExtensionsModal', () => ({
  default: () => <div role="dialog" aria-label="Browse extensions" />,
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
 * The page on the shared band (spec 3.11 / 3.10), in the DOM.
 *
 * ⚠ Asserted through the DOM rather than by grepping the source, for the reason
 * `schedule/SchedulesView.test.tsx` records: a source grep for `<PageHeader` passes on a view
 * that mounts the shared header AND has quietly grown a second title or a stray action beside
 * it. What jsdom cannot see (the 44px, the hairline, the drag rect) is measured in Chromium by
 * `Layout/pageBandGeometry.browser.test.tsx`.
 */
describe('ExtensionsView — the band', () => {
  beforeEach(() => {
    mocks.extensionsList = [];
    // `platform` is set rather than left absent so the shortcut assertions name a branch on
    // purpose instead of relying on jsdom happening to have no `window.electron` at all.
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
  const filterField = () => screen.getByRole('searchbox', { name: EXTENSIONS_COPY.filterLabel });

  it('has exactly one page title, in the band', async () => {
    renderView();

    const heading = await screen.findByRole('heading', { level: 1, name: 'Extensions' });
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    // The 2026-09-07 decision ("actions on their own line, nothing clickable on the title's
    // row") is reversed (spec 6.6): the band holds the title, its help and the actions on one
    // 44px line, so a button beside the title is now correct, not a defect.
    expect(screen.getByTestId('page-header')).toContainElement(heading);
  });

  it('keeps the explanation behind an InfoTip, with the shortcut on the filter instead', async () => {
    renderView();

    const info = await screen.findByRole('button', { name: 'About Extensions' });
    expect(info).toHaveAccessibleDescription(EXTENSIONS_COPY.info);
    expect(EXTENSIONS_COPY.info).toBe(
      'Extensions add tools, prompts and resources. Enabled ones apply to every new chat.'
    );
    // No visible paragraph, and no "to search" sentence: the shortcut is the field's tooltip.
    expect(screen.queryByText(/to search/)).toBeNull();
    expect(screen.queryByText(/MCP extensions expand/)).toBeNull();
  });

  it('shows the filter in the band, and ⌘F / Ctrl+F focuses it', async () => {
    renderView();
    await screen.findByRole('heading', { level: 1, name: 'Extensions' });

    const field = filterField();
    expect(screen.getByTestId('page-header')).toContainElement(field);
    expect(field).not.toHaveFocus();

    const mac = isMacPlatform();
    fireEvent.keyDown(window, { key: 'f', metaKey: mac, ctrlKey: !mac });
    expect(field).toHaveFocus();
  });

  it('names the filter for what it filters, as its siblings do, and the tooltip starts the same way', async () => {
    const user = userEvent.setup({ delay: null });
    renderView();
    await screen.findByRole('heading', { level: 1, name: 'Extensions' });

    // Skills and Workflows say "Filter skills" / "Filter workflows"; a bare "Filter" read the
    // same on every page (vision round 1, V3-25).
    expect(EXTENSIONS_COPY.filterLabel).toBe('Filter extensions');
    expect(filterField()).toHaveAccessibleName('Filter extensions');

    await user.hover(filterField());
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent(uiCopy.filterTooltip(EXTENSIONS_COPY.filterLabel));
    expect(tooltip.textContent).toMatch(/^Filter extensions · /);
  });

  it('puts one Add menu in the band instead of three buttons', async () => {
    const user = userEvent.setup({ delay: null });
    renderView();

    const actions = (await screen.findByTestId('page-header')).querySelector(
      '.biorouter-page-header-actions'
    ) as HTMLElement;
    expect(actions).not.toBeNull();
    const add = within(actions).getByRole('button', { name: EXTENSIONS_COPY.add });
    // The three old labelled buttons are gone; their jobs are the menu's items.
    for (const gone of ['Add extension', 'Browse extensions', 'Add custom extension']) {
      expect(screen.queryByRole('button', { name: gone })).toBeNull();
    }

    await user.click(add);
    expect(screen.getAllByRole('menuitem').map((item) => item.textContent)).toEqual([
      EXTENSIONS_COPY.browse,
      EXTENSIONS_COPY.fromFile,
      EXTENSIONS_COPY.custom,
    ]);
  });

  /**
   * V7. The three buttons carried `className="flex items-center gap-2"`, and a bare `flex`
   * FLIPS `buttonVariants`' own `inline-flex` through tailwind-merge. jsdom runs no Tailwind, so
   * the class STRING is the only observable: the Add button must not carry it.
   */
  it('lets the Button primitive own its own layout', async () => {
    renderView();
    const add = await screen.findByRole('button', { name: EXTENSIONS_COPY.add });
    expect(add.className.split(/\s+/)).not.toContain('flex');
  });

  it('opens the custom, file and marketplace dialogs from the menu', async () => {
    const user = userEvent.setup({ delay: null });
    renderView();

    await user.click(await screen.findByRole('button', { name: EXTENSIONS_COPY.add }));
    await user.click(screen.getByRole('menuitem', { name: EXTENSIONS_COPY.custom }));
    expect(
      await screen.findByRole('dialog', { name: EXTENSIONS_COPY.customTitle })
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    await user.click(screen.getByRole('button', { name: EXTENSIONS_COPY.add }));
    await user.click(screen.getByRole('menuitem', { name: EXTENSIONS_COPY.fromFile }));
    expect(await screen.findByRole('dialog', { name: 'Add extension' })).toBeInTheDocument();
  });

  it('opens the marketplace from the menu', async () => {
    const user = userEvent.setup({ delay: null });
    renderView();

    await user.click(await screen.findByRole('button', { name: EXTENSIONS_COPY.add }));
    await user.click(screen.getByRole('menuitem', { name: EXTENSIONS_COPY.browse }));
    expect(await screen.findByRole('dialog', { name: 'Browse extensions' })).toBeInTheDocument();
  });

  /**
   * The band is the window's drag region on this route (`routeOwnsTopBand`), so it must not
   * scroll away with a long list: it sits outside the scroller, which holds the rows.
   */
  it('keeps the band outside the scroller, so a long list never scrolls it away', async () => {
    mocks.extensionsList = [
      { type: 'stdio', name: 'medcp', description: 'Local MCP', enabled: true },
    ];
    renderView();

    const scroller = await screen.findByTestId('list-scroller');
    const band = screen.getByTestId('page-header');
    expect(scroller).not.toContainElement(band);
    expect(scroller).toContainElement(await screen.findByText('Local MCP'));
  });

  it('counts the listed extensions in the band, and leaves out the shipped capabilities', async () => {
    mocks.extensionsList = [
      { type: 'stdio', name: 'medcp', description: 'Local MCP', enabled: true },
      { type: 'stdio', name: 'spokeagent', description: 'SPOKE graph', enabled: false },
      // A shipped capability has no row here, so it is not counted.
      { type: 'platform', name: 'Workspace', description: 'Workspace Control', enabled: true },
    ];
    renderView();

    const band = await screen.findByTestId('page-header');
    expect(band.querySelector('.biorouter-page-header-adornment')).toHaveTextContent(/^2$/);
  });

  it('filters the rows from the band, and says so when nothing matches', async () => {
    mocks.extensionsList = [
      { type: 'stdio', name: 'medcp', description: 'Local MCP', enabled: true },
      { type: 'stdio', name: 'spokeagent', description: 'SPOKE graph', enabled: true },
    ];
    renderView();
    await screen.findByText('Local MCP');

    fireEvent.change(filterField(), { target: { value: 'spoke' } });
    expect(screen.getByText('SPOKE graph')).toBeInTheDocument();
    expect(screen.queryByText('Local MCP')).toBeNull();

    fireEvent.change(filterField(), { target: { value: 'zzz' } });
    expect(
      await screen.findByRole('region', { name: 'No matching extensions' })
    ).toBeInTheDocument();
  });
});
