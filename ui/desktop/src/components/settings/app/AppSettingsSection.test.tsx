import { afterEach, describe, expect, it, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { renderToString } from 'react-dom/server';
import { aboutCopy, appearanceCopy, generalCopy } from './copy';

// The General rows are what this file is about; the rest of the App tab is
// other sections' business and pulls in the API client, the theme registry and
// the usage panel.
vi.mock('./UpdateSection', () => ({ default: () => null }));
vi.mock('../usage/UsageSection', () => ({
  default: ({ showCosts }: { showCosts?: boolean }) => (
    <div data-testid="usage-section" data-show-costs={String(showCosts)} />
  ),
}));
vi.mock('./ResetPanel', () => ({ default: () => null }));
vi.mock('../../BioRouterSidebar/ThemeSelector', () => ({ default: () => null }));
vi.mock('../../BioRouterSidebar/ThemeFamilySelector', () => ({ default: () => null }));
vi.mock('./FontSizeSelector', () => ({ default: () => null }));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: vi.fn(async () => false), upsert: vi.fn(async () => undefined) }),
}));

import AppSettingsSection from './AppSettingsSection';

beforeEach(() => {
  Object.assign(window, {
    electron: {
      platform: 'darwin',
      getMenuBarIconState: vi.fn().mockResolvedValue(true),
      getDockIconState: vi.fn().mockResolvedValue(true),
      getWakelockState: vi.fn().mockResolvedValue(true),
      setMenuBarIcon: vi.fn().mockResolvedValue(true),
      setDockIcon: vi.fn().mockResolvedValue(true),
      setWakelock: vi.fn().mockResolvedValue(true),
      openNotificationsSettings: vi.fn(),
    },
    appConfig: { get: vi.fn().mockReturnValue(undefined) },
  });
});

/**
 * Each switch's accessible name IS its visible row label, word for word (spec 2.6): someone
 * driving the app by voice says what they can read. Before the rows had one anatomy, all four
 * came back with no name at all and a screen reader announced "switch, on" with no subject.
 */
describe('General switches', () => {
  it.each([
    [generalCopy.menuBar],
    [generalCopy.dock],
    [generalCopy.preventSleep],
    [generalCopy.announceOnly],
    [generalCopy.showCosts],
  ])('names the "%s" switch by its label', async (name) => {
    render(<AppSettingsSection />);
    expect(await screen.findByRole('switch', { name })).toHaveAccessibleName(name);
  });

  it('names every switch in the panel, so none is left to be found by position', async () => {
    render(<AppSettingsSection />);
    await screen.findByRole('switch', { name: generalCopy.menuBar });
    const unnamed = screen
      .getAllByRole('switch')
      .filter(
        (control) => !control.getAttribute('aria-labelledby') && !control.getAttribute('aria-label')
      );
    expect(unnamed).toEqual([]);
  });

  it('keeps the explanation as help the switch hears, not a paragraph', async () => {
    const { container } = render(<AppSettingsSection />);
    const sleep = await screen.findByRole('switch', { name: generalCopy.preventSleep });
    expect(sleep).toHaveAccessibleDescription(generalCopy.preventSleepHelp);
    // No description paragraph under any row label.
    expect(container.querySelectorAll('.biorouter-settings-row p')).toHaveLength(0);
  });
});

/**
 * The three switches whose value the main process holds are read after the page mounts. Until it
 * answers they are held at their place and not painted, so a person who turned "Prevent sleep"
 * off never sees it on (vision V4-01: the coral ON track showed for two frames on every open of
 * the App tab, then snapped to off).
 */
describe('switches read from the main process', () => {
  /** The switch in the row with this label, whether or not it is in the accessibility tree yet. */
  function switchInRow(label: string): HTMLElement {
    const row = Array.from(document.querySelectorAll('[data-slot="setting-row"]')).find(
      (candidate) => candidate.querySelector('label')?.textContent === label
    );
    return row!.querySelector('[data-slot="switch"]') as HTMLElement;
  }

  /** Every `aria-checked` the named switch carried from the first render on. */
  function watchChecked(element: HTMLElement) {
    const seen = new Set([element.getAttribute('aria-checked')]);
    const observer = new MutationObserver(() => seen.add(element.getAttribute('aria-checked')));
    observer.observe(element, { attributes: true, attributeFilter: ['aria-checked'] });
    return { seen, stop: () => observer.disconnect() };
  }

  it('never paints a stored "off" as on, and only offers the switch once it knows', async () => {
    let answer: (enabled: boolean) => void = () => {};
    window.electron.getWakelockState = vi.fn(
      () => new Promise<boolean>((resolve) => (answer = resolve))
    );
    render(<AppSettingsSection />);

    // The switch is in the row from the first render, holding its place.
    const held = switchInRow(generalCopy.preventSleep);
    expect(held).toHaveAttribute('data-pending');
    expect(screen.queryByRole('switch', { name: generalCopy.preventSleep })).toBeNull();
    expect(held).toHaveAttribute('aria-checked', 'false');
    const record = watchChecked(held);

    await act(async () => answer(false));

    const sleep = await screen.findByRole('switch', { name: generalCopy.preventSleep });
    expect(sleep).toBe(held);
    expect(sleep).not.toBeChecked();
    expect(record.seen).toEqual(new Set(['false']));
    record.stop();
  });

  it('shows a stored "on" as on once it is read, as the same node', async () => {
    let answer: (enabled: boolean) => void = () => {};
    window.electron.getWakelockState = vi.fn(
      () => new Promise<boolean>((resolve) => (answer = resolve))
    );
    render(<AppSettingsSection />);
    const held = switchInRow(generalCopy.preventSleep);
    await act(async () => answer(true));
    expect(await screen.findByRole('switch', { name: generalCopy.preventSleep })).toBe(held);
    expect(held).toBeChecked();
  });

  it('leaves the setting at its default, on, when its state cannot be read', async () => {
    window.electron.getWakelockState = vi.fn().mockRejectedValue(new Error('no main process'));
    render(<AppSettingsSection />);
    expect(await screen.findByRole('switch', { name: generalCopy.preventSleep })).toBeChecked();
  });

  it('holds all four, Dock and Never open tabs included, until each has been read', async () => {
    render(<AppSettingsSection />);
    expect(document.querySelectorAll('[data-pending]')).toHaveLength(4);
    for (const name of [generalCopy.menuBar, generalCopy.dock, generalCopy.preventSleep]) {
      expect(await screen.findByRole('switch', { name })).toBeChecked();
    }
    expect(await screen.findByRole('switch', { name: generalCopy.announceOnly })).not.toBeChecked();
    expect(document.querySelectorAll('[data-pending]')).toHaveLength(0);
  });
});

/**
 * Settings mounts only the tab on screen, so every visit to App is a new mount. A switch that
 * starts from "not read yet" is held unpainted for the one or two frames the main process takes,
 * on every visit, and the column of switches fills in two steps (vision round 8, V4-R7-2). The
 * answer cannot have changed since the last visit, so a later one starts from it.
 */
describe('a later visit to the App tab', () => {
  /** The held switches never answer, so only a remembered value can paint them. */
  function hangIpc() {
    const never = () => new Promise<boolean>(() => {});
    window.electron.getMenuBarIconState = vi.fn(never);
    window.electron.getDockIconState = vi.fn(never);
    window.electron.getWakelockState = vi.fn(never);
  }

  const mainProcessSwitches = [generalCopy.menuBar, generalCopy.dock, generalCopy.preventSleep];

  /** The first visit, with distinct answers, left again as a tab switch leaves it. */
  async function firstVisit() {
    window.electron.getMenuBarIconState = vi.fn().mockResolvedValue(false);
    window.electron.getDockIconState = vi.fn().mockResolvedValue(true);
    window.electron.getWakelockState = vi.fn().mockResolvedValue(false);
    const visit = render(<AppSettingsSection />);
    for (const name of mainProcessSwitches) await screen.findByRole('switch', { name });
    visit.unmount();
  }

  it('paints every main-process switch, with its last value, in the first render', async () => {
    await firstVisit();
    hangIpc();
    render(<AppSettingsSection />);

    // Nothing is held, and nothing has been awaited: this is the first committed frame.
    expect(document.querySelectorAll('[data-pending]')).toHaveLength(0);
    expect(screen.getByRole('switch', { name: generalCopy.menuBar })).not.toBeChecked();
    expect(screen.getByRole('switch', { name: generalCopy.dock })).toBeChecked();
    expect(screen.getByRole('switch', { name: generalCopy.preventSleep })).not.toBeChecked();
  });

  it('still asks the main process, and shows what it says when it differs', async () => {
    await firstVisit();
    window.electron.getWakelockState = vi.fn().mockResolvedValue(true);
    render(<AppSettingsSection />);
    expect(screen.getByRole('switch', { name: generalCopy.preventSleep })).not.toBeChecked();
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: generalCopy.preventSleep })).toBeChecked()
    );
    expect(window.electron.getWakelockState).toHaveBeenCalledTimes(1);
  });

  it('starts from a change the person made, not the answer before it', async () => {
    await firstVisit();
    const first = render(<AppSettingsSection />);
    fireEvent.click(await screen.findByRole('switch', { name: generalCopy.preventSleep }));
    await waitFor(() => expect(window.electron.setWakelock).toHaveBeenCalledWith(true));
    first.unmount();

    hangIpc();
    render(<AppSettingsSection />);
    expect(screen.getByRole('switch', { name: generalCopy.preventSleep })).toBeChecked();
  });

  it('keeps the last answer when a later read fails, and shows no default over it', async () => {
    await firstVisit();
    window.electron.getMenuBarIconState = vi.fn().mockRejectedValue(new Error('no main process'));
    render(<AppSettingsSection />);
    await waitFor(() => expect(window.electron.getMenuBarIconState).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.getByRole('switch', { name: generalCopy.menuBar })).not.toBeChecked();
  });

  it('holds a switch nobody has read, rather than painting a guess, on a first visit', async () => {
    hangIpc();
    render(<AppSettingsSection />);
    expect(document.querySelectorAll('[data-pending]').length).toBeGreaterThanOrEqual(3);
    expect(screen.queryByRole('switch', { name: generalCopy.menuBar })).toBeNull();
    // Let the config-backed rows' reads land inside act.
    await act(async () => {});
  });
});

/**
 * A server render runs no effects, so it is the first paint. What the platform and storage
 * already know must be on it: a row that arrives one frame later shifts every row below it.
 */
describe('the first render', () => {
  it('already has the macOS-only Dock row and the macOS spelling of the Notifications button', () => {
    const html = renderToString(<AppSettingsSection />);
    expect(html).toContain(generalCopy.dock);
    expect(html).toContain(generalCopy.openNotificationsMac);
  });

  it('leaves the Dock row out on another platform from the first render', () => {
    (window.electron as unknown as { platform: string }).platform = 'linux';
    const html = renderToString(<AppSettingsSection />);
    expect(html).not.toContain(generalCopy.dock);
    expect(html).toContain(generalCopy.openNotificationsOther);
  });
});

/**
 * Show costs hides the dollars, not the section. Unmounting Usage dropped 110px from the page, so
 * a person scrolled near the bottom had every row move 72px under the pointer and their next
 * click land on Prevent sleep (vision V4-02).
 */
describe('Show costs', () => {
  afterEach(() => localStorage.clear());

  it('keeps the Usage section mounted, the same node, and tells it to drop its costs', async () => {
    localStorage.clear();
    render(<AppSettingsSection />);
    const showCosts = await screen.findByRole('switch', { name: generalCopy.showCosts });
    const usage = screen.getByTestId('usage-section');
    expect(usage).toHaveAttribute('data-show-costs', 'true');

    fireEvent.click(showCosts);
    expect(showCosts).not.toBeChecked();
    expect(localStorage.getItem('show_pricing')).toBe('false');
    expect(screen.getByTestId('usage-section')).toBe(usage);
    expect(usage).toHaveAttribute('data-show-costs', 'false');

    fireEvent.click(showCosts);
    expect(screen.getByTestId('usage-section')).toBe(usage);
    expect(usage).toHaveAttribute('data-show-costs', 'true');
  });

  it('starts with costs hidden when the person hid them, on the first render', () => {
    localStorage.setItem('show_pricing', 'false');
    const html = renderToString(<AppSettingsSection />);
    expect(html).toContain('data-testid="usage-section"');
    expect(html).toContain('data-show-costs="false"');
  });
});

describe('App tab sections', () => {
  it('orders General, Appearance, About', async () => {
    render(<AppSettingsSection />);
    await screen.findByRole('switch', { name: generalCopy.menuBar });
    const headings = screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent);
    expect(headings).toEqual([generalCopy.section, appearanceCopy.section, aboutCopy.section]);
  });

  it('opens the notification settings from a row button named by its own words', async () => {
    render(<AppSettingsSection />);
    const open = await screen.findByRole('button', { name: generalCopy.openNotificationsMac });
    open.click();
    expect(window.electron.openNotificationsSettings).toHaveBeenCalledTimes(1);
  });

  it('shows a pinned build’s version as a row value, with no update control', async () => {
    (window.appConfig.get as ReturnType<typeof vi.fn>).mockImplementation((key: string) =>
      key === 'BIOROUTER_VERSION' ? '1.92.1' : undefined
    );
    render(<AppSettingsSection />);
    expect(await screen.findByText('1.92.1')).toBeInTheDocument();
    // About draws no logo today. The only one it may ever show is the Biorouter wordmark
    // (R-29; its svg is `role="img"` named "BioRouter"), so an image of any other name fails.
    for (const logo of screen.queryAllByRole('img')) {
      expect(logo).toHaveAccessibleName('BioRouter');
    }
  });
});

/**
 * Report a bug and Request a feature send the person to GitHub in their default browser, so they
 * say so the way every other exit in Settings does (vision V4-R6-2): the 14px external-link glyph
 * after the label, and the description "Opens in your browser" for a person who cannot see it.
 * jsdom lays nothing out, so the glyph's size, place and ink are read in Chromium by
 * `FeedbackLinks.browser.test.tsx`.
 */
describe('Feedback exits', () => {
  const exits = [
    [aboutCopy.reportBug, 'template=bug_report.md'],
    [aboutCopy.requestFeature, 'template=feature_request.md'],
  ] as const;

  it.each(exits)('marks "%s" as leaving the app, by name and by description', async (name) => {
    render(<AppSettingsSection />);
    const exit = await screen.findByRole('button', { name });
    // The glyph is decoration, so the name is still the visible words and nothing more.
    expect(exit).toHaveAccessibleName(name);
    expect(exit).toHaveAccessibleDescription(aboutCopy.opensInBrowser);
    const glyph = exit.querySelector('svg');
    expect(glyph).not.toBeNull();
    expect(glyph).toHaveAttribute('aria-hidden', 'true');
    // After the label, not before it.
    expect(exit.lastElementChild).toBe(glyph);
    // `size-` keeps the button primitive from drawing it at 16px.
    expect(glyph!.getAttribute('class')).toMatch(/\bsize-3\.5\b/);
  });

  it.each(exits)('still opens the right GitHub form from "%s"', async (name, template) => {
    const open = vi.spyOn(window, 'open').mockImplementation(() => null);
    try {
      render(<AppSettingsSection />);
      fireEvent.click(await screen.findByRole('button', { name }));
      expect(open).toHaveBeenCalledTimes(1);
      expect(open.mock.calls[0][0]).toContain(template);
      expect(open.mock.calls[0][1]).toBe('_blank');
    } finally {
      open.mockRestore();
    }
  });

  it('keeps the group the row names, with one hidden description for both exits', async () => {
    const { container } = render(<AppSettingsSection />);
    const reportBug = await screen.findByRole('button', { name: aboutCopy.reportBug });
    expect(reportBug.closest('[role="group"]')).toHaveAccessibleName(aboutCopy.feedback);
    // Hidden, so it takes no room in the row and is not read as a line of text.
    const hidden = Array.from(container.querySelectorAll('[hidden]')).filter(
      (node) => node.textContent === aboutCopy.opensInBrowser
    );
    expect(hidden).toHaveLength(1);
  });
});
