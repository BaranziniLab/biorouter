import { afterEach, describe, expect, it, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
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
    expect(screen.queryByAltText(/Block/)).toBeNull();
  });
});
