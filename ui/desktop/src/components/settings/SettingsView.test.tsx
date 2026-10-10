import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SettingsView from './SettingsView';
import { CONFIGURATION_ENABLED } from '../../updates';
import { SETTINGS_SECTION_IDS } from './settingsSections';
import { settingsShellCopy } from './app/copy';

// Every section but Privacy is stubbed, and each stub is identifiable, so this
// file can assert both that the Privacy panel is really mounted (the failure
// mode being a settings component that is declared and plausible but has zero
// consumers repo-wide, so it renders for nobody) and WHERE it sits relative to
// its neighbours.
vi.mock('./models/ModelsSection', () => ({
  default: () => <div data-testid="section-models" />,
}));
vi.mock('./chat/ChatSettingsSection', () => ({
  default: () => (
    <div data-testid="section-chat">
      <section id={SETTINGS_SECTION_IDS.approvals} data-testid="section-approvals" />
    </div>
  ),
}));
vi.mock('./app/AppSettingsSection', () => ({
  default: () => <div data-testid="section-app" />,
}));
vi.mock('./config/ConfigSettings', () => ({
  default: () => <div data-testid="section-config" />,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    read: vi.fn(async () => undefined),
    upsert: vi.fn(async () => undefined),
  }),
}));

const renderSettings = (viewOptions = {}) =>
  render(<SettingsView onClose={() => {}} setView={() => {}} viewOptions={viewOptions} />);

/** Document order of the sections that actually rendered. */
function sectionOrder(): string[] {
  return [...document.querySelectorAll('[data-testid^="section-"], [data-privacy-panel]')].map(
    (el) => el.getAttribute('data-testid') ?? 'section-privacy'
  );
}

const scrollIntoView = vi.fn();

beforeEach(() => {
  scrollIntoView.mockReset();
  Element.prototype.scrollIntoView = scrollIntoView;
});

afterEach(() => {
  vi.useRealTimers();
});

describe('SettingsView', () => {
  /**
   * ⚠ **Privacy is a SECTION of App, not a tab.** It shipped as a fourth tab,
   * which made it read as a separate product rather than a property of this
   * install. There is no `settings-privacy-tab` any more, and a test that
   * clicked one would fail loudly rather than quietly asserting nothing.
   */
  it('mounts the Privacy panel inside the App tab, with no tab of its own', async () => {
    const user = userEvent.setup();
    renderSettings();
    expect(screen.queryByTestId('settings-privacy-tab')).toBeNull();

    await user.click(screen.getByTestId('settings-app-tab'));
    expect(await screen.findByRole('switch', { name: /Privacy tiers/ })).toBeInTheDocument();
  });

  /**
   * The operator's order, and it is the point of the change: Configuration,
   * Privacy, then everything `AppSettingsSection` owns (General, which now holds
   * the old one-row Workspace section, Appearance, Usage, About, Danger zone).
   *
   * Asserted as document order rather than by eyeballing the JSX, because the
   * JSX is exactly what a later edit reorders.
   */
  it('orders the App tab: Configuration, Privacy, then the rest', async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(screen.getByTestId('settings-app-tab'));
    await screen.findByRole('switch', { name: /Privacy tiers/ });

    const order = sectionOrder();
    const expected = CONFIGURATION_ENABLED
      ? ['section-config', 'section-privacy', 'section-app']
      : ['section-privacy', 'section-app'];
    expect(order).toEqual(expected);
  });

  /**
   * ⚠ A deep link to `section: 'privacy'` predates the move and must still land
   * somewhere it exists. Selecting a tab value that no longer has a trigger
   * leaves the whole panel blank, which is the shape of this bug.
   */
  it('still honours an old deep link to the privacy section', async () => {
    renderSettings({ section: 'privacy' });
    expect(await screen.findByRole('switch', { name: /Privacy tiers/ })).toBeInTheDocument();
  });
});

describe('the Settings band', () => {
  /**
   * One 44px band holds the title and the three tabs (spec §3.13): no description paragraph,
   * no tab strip of its own under the header, and text-only tabs.
   */
  it('holds the title and the three text-only tabs, and no description', () => {
    renderSettings();
    const band = screen.getByTestId('page-header');
    expect(within(band).getByRole('heading', { level: 1 })).toHaveTextContent(
      settingsShellCopy.title
    );
    const tabs = within(band).getAllByRole('tab');
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      settingsShellCopy.tabs.models,
      settingsShellCopy.tabs.chat,
      settingsShellCopy.tabs.app,
    ]);
    for (const tab of tabs) expect(tab.querySelector('svg')).toBeNull();
    expect(within(band).queryByText(/Manage models/)).toBeNull();
  });

  it('keeps the tab test ids the e2e suite reads', () => {
    renderSettings();
    for (const id of ['settings-models-tab', 'settings-chat-tab', 'settings-app-tab']) {
      expect(screen.getByTestId(id)).toBeInTheDocument();
    }
  });

  it('opens on Models, or on the tab a deep link names', () => {
    const { unmount } = renderSettings();
    expect(screen.getByTestId('settings-models-tab')).toHaveAttribute('data-state', 'active');
    unmount();
    renderSettings({ section: 'modes' });
    expect(screen.getByTestId('settings-chat-tab')).toHaveAttribute('data-state', 'active');
  });
});

describe('Settings scrolling', () => {
  const viewport = () =>
    document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]') as HTMLElement;

  /** One scroller holds all three tabs; a tab change starts the next tab at its top. */
  it('starts each tab at the top', async () => {
    const user = userEvent.setup();
    renderSettings();
    viewport().scrollTop = 600;
    await user.click(screen.getByTestId('settings-chat-tab'));
    expect(viewport().scrollTop).toBe(0);
  });

  it('scrolls a deep-linked section into view and highlights it', async () => {
    vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'setTimeout', 'clearTimeout'] });
    renderSettings({ section: 'modes' });
    const section = screen.getByTestId('section-approvals');
    await act(async () => {
      vi.advanceTimersByTime(50);
    });
    expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ block: 'start' }));
    expect(scrollIntoView.mock.contexts[0]).toBe(section);
    expect(section).toHaveClass('br-highlight');
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(section).not.toHaveClass('br-highlight');
  });

  it('jumps instead of gliding when the person prefers reduced motion', async () => {
    const matchMedia = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: query.includes('reduce'),
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      onchange: null,
      dispatchEvent: () => false,
    })) as typeof window.matchMedia;
    try {
      vi.useFakeTimers({ toFake: ['requestAnimationFrame'] });
      renderSettings({ section: 'modes' });
      await act(async () => {
        vi.advanceTimersByTime(50);
      });
      expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ behavior: 'auto' }));
    } finally {
      window.matchMedia = matchMedia;
    }
  });
});

/**
 * Settings reads the CHAT measure (760px), not the fluid page measure
 * (operator decision, 2026-09-07): it is a column of labelled rows, so width
 * beyond the measure separates each control from the label it names instead of
 * showing more.
 *
 * ⚠ **What this can and cannot see.** jsdom has no layout engine and never runs
 * Tailwind, so the WIDTH is unassertable here; what is assertable is the
 * attribute that selects it. The band is not a reading column (it runs edge to
 * edge), so the body is the one column, and it is pinned so that a second
 * column added on the default size fails here rather than shipping a step in
 * the left edge. `styles/measures.test.ts` closes the remaining case this file
 * cannot see: a `<ReadableContent` written with no `size` prop at all.
 */
describe('SettingsView sits on the chat measure', () => {
  const readableColumns = () =>
    [...document.querySelectorAll('.biorouter-readable-content')] as HTMLElement[];

  it('renders one reading column, on the chat size', () => {
    renderSettings();

    const columns = readableColumns();
    expect(columns).toHaveLength(1);
    for (const column of columns) expect(column.dataset.size).toBe('chat');
  });

  it('keeps the column on the chat size after switching tabs', async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(screen.getByTestId('settings-app-tab'));
    await screen.findByRole('switch', { name: /Privacy tiers/ });

    const columns = readableColumns();
    expect(columns).toHaveLength(1);
    for (const column of columns) expect(column.dataset.size).toBe('chat');
  });

  /**
   * W2-PRV-11. Radix dismisses its dialogs on a capture-phase Escape and marks
   * the event handled with preventDefault(); the Settings listener used to
   * leave Settings on that same key press, so closing a modal also went Home.
   */
  it('leaves Settings on a bare Escape, but not on one a dialog already handled', () => {
    const onClose = vi.fn();
    render(<SettingsView onClose={onClose} setView={() => {}} viewOptions={{}} />);

    const handledByDialog = (event: KeyboardEvent) => event.preventDefault();
    document.addEventListener('keydown', handledByDialog, { capture: true });
    document.body.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    );
    document.removeEventListener('keydown', handledByDialog, { capture: true });
    expect(onClose).not.toHaveBeenCalled();

    document.body.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
