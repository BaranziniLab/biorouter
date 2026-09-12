import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// The Appearance rows are what this file is about; the rest of the App tab is
// other sections' business and pulls in the API client, the theme registry and
// the usage panel.
vi.mock('./UpdateSection', () => ({ default: () => null }));
vi.mock('../usage/UsageSection', () => ({ default: () => null }));
vi.mock('./ResetPanel', () => ({ default: () => null }));
vi.mock('../../BioRouterSidebar/ThemeSelector', () => ({ default: () => null }));
vi.mock('../../BioRouterSidebar/ThemeFamilySelector', () => ({ default: () => null }));

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
 * Measured with the accessibility tree: all four Appearance switches came back
 * with `aria-label` null, `aria-labelledby` null and no text content, so a
 * screen reader announced "switch, on" with no subject — four times in one
 * panel, each about something different.
 *
 * The subject is on screen; it is the `<p>` beside the switch. It just was not
 * connected to the control, which is exactly what the Privacy tiers switch two
 * panels over does connect (`aria-label="Privacy tiers"`), and this follows it.
 *
 * The name is the visible label verbatim, not a paraphrase: someone driving the
 * app by voice says what they can read.
 */
describe('Appearance switches', () => {
  it.each([['Menu bar icon'], ['Dock icon'], ['Prevent sleep'], ['Cost tracking']])(
    'announces what "%s" is about',
    async (name) => {
      render(<AppSettingsSection />);
      expect(await screen.findByRole('switch', { name })).toHaveAccessibleName(name);
    }
  );

  it('names every switch in the panel, so none is left to be found by position', async () => {
    render(<AppSettingsSection />);
    await screen.findByRole('switch', { name: 'Menu bar icon' });
    const unnamed = screen
      .getAllByRole('switch')
      .filter((control) => !control.getAttribute('aria-label')?.trim());
    expect(unnamed).toEqual([]);
  });
});
