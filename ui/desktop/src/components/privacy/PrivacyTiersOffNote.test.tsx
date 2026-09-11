import { readFileSync } from 'node:fs';
import path from 'node:path';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PrivacyTiersOffNote } from './PrivacyTiersOffNote';
import type { PrivacyTiersRecord } from '../settings/privacy/privacyTiers';

/**
 * H3 (2026-09-10 security test drive). The switch's record is agent-writable —
 * DR-17's accepted risk — and a `{"enabled": false}` written from a chat's shell
 * turned every gate off at the next launch with nothing in the app saying so.
 * This note is the in-app half of the fix: it stands above the composer for as
 * long as the tiers are off, and says where the switch was recorded and whether
 * the app recorded turning it off.
 *
 * The two hooks are the seam, as in `PrivacyBadge.test.tsx`: mounting a real
 * `ConfigProvider` would drag in the daemon client, the provider list and the
 * extension sync.
 */
const configMocks = vi.hoisted(() => ({
  enabled: true,
  record: null as PrivacyTiersRecord | null,
}));
vi.mock('../ConfigContext', () => ({
  usePrivacyTiersEnabled: () => configMocks.enabled,
  usePrivacyTiersRecord: () => configMocks.record,
}));

const PATH = '/Users/someone/.config/biorouter/privacy-tiers.json';

const record = (overrides: Partial<PrivacyTiersRecord>): PrivacyTiersRecord => ({
  enabled: false,
  origin: 'unrecorded',
  path: PATH,
  lastChange: null,
  ...overrides,
});

/** Where a click on the note's one control took the app. */
function Location() {
  const location = useLocation();
  return (
    <p data-testid="location">
      {location.pathname} {JSON.stringify(location.state)}
    </p>
  );
}

const mount = () =>
  render(
    <MemoryRouter initialEntries={['/']}>
      <Routes>
        <Route path="/" element={<PrivacyTiersOffNote />} />
        <Route path="/settings" element={<Location />} />
      </Routes>
    </MemoryRouter>
  );

beforeEach(() => {
  configMocks.enabled = true;
  configMocks.record = null;
});
afterEach(cleanup);

describe('PrivacyTiersOffNote', () => {
  it('renders nothing while privacy tiers are on', () => {
    configMocks.record = record({ enabled: true, origin: 'settings' });
    mount();
    expect(screen.queryByTestId('privacy-tiers-off-note')).toBeNull();
  });

  it('stands while privacy tiers are off, and says where the switch is recorded', () => {
    configMocks.enabled = false;
    configMocks.record = record({ origin: 'unrecorded' });
    mount();

    const note = screen.getByTestId('privacy-tiers-off-note');
    expect(note).toHaveTextContent(/Privacy tiers are off/i);
    expect(note).toHaveTextContent(PATH);
    // A standing condition is `status`, not `alert` — nothing just failed.
    expect(note).toHaveAttribute('role', 'status');
  });

  it('says the switch was turned off outside the app when no deliberate change is recorded', () => {
    configMocks.enabled = false;
    configMocks.record = record({ origin: 'unrecorded' });
    mount();

    expect(screen.getByTestId('privacy-tiers-off-note')).toHaveTextContent(
      /turned off outside the app/i
    );
  });

  it('names the last recorded change when a one-field edit contradicts it', () => {
    configMocks.enabled = false;
    configMocks.record = record({
      origin: 'unrecorded',
      lastChange: {
        via: 'settings',
        setTo: true,
        at: '2026-09-10T18:04:00+00:00',
        systemAuthenticated: false,
        userAction: true,
      },
    });
    mount();

    expect(screen.getByTestId('privacy-tiers-off-note')).toHaveTextContent(
      /last change recorded in the app turned them on/i
    );
  });

  it('does not accuse anyone when the change was made in Settings → Privacy', () => {
    configMocks.enabled = false;
    configMocks.record = record({
      origin: 'settings',
      lastChange: {
        via: 'settings',
        setTo: false,
        at: '2026-09-10T18:04:00+00:00',
        systemAuthenticated: true,
        userAction: true,
      },
    });
    mount();

    const note = screen.getByTestId('privacy-tiers-off-note');
    expect(note).toHaveTextContent(/Settings → Privacy/);
    expect(note).not.toHaveTextContent(/outside the app/i);
    expect(note).toHaveTextContent(PATH);
  });

  it('still stands when the daemon sends no record at all', () => {
    // An older daemon behind the external-backend setup serves the switch but
    // not the record. The note must not need the record to exist.
    configMocks.enabled = false;
    configMocks.record = null;
    mount();

    expect(screen.getByTestId('privacy-tiers-off-note')).toHaveTextContent(
      /Privacy tiers are off/i
    );
  });

  it('has no dismiss control, and its one control opens Settings → Privacy', async () => {
    const user = userEvent.setup();
    configMocks.enabled = false;
    configMocks.record = record({ origin: 'unrecorded' });
    mount();

    const note = screen.getByTestId('privacy-tiers-off-note');
    const controls = Array.from(note.querySelectorAll('button'));
    expect(controls).toHaveLength(1);
    expect(controls[0]).not.toHaveAccessibleName(/dismiss|close|hide/i);

    await user.click(controls[0]);
    expect(screen.getByTestId('location')).toHaveTextContent('/settings {"section":"privacy"}');
  });

  /**
   * A note nobody mounts is the same silence the drive measured. Asserted at the
   * source, as `PrivacyBadge.test.tsx` does for its own call sites, because
   * neither surface mounts cheaply in jsdom. Home is the load-bearing one: it
   * is the route the app LAUNCHES on, and a switch turned off outside the app
   * takes effect at a launch.
   */
  it('is mounted above both composers the app has: every chat, and Home', () => {
    // vitest runs with `ui/desktop` as its root.
    const source = (file: string) => readFileSync(path.join(process.cwd(), file), 'utf8');
    for (const file of ['src/components/BaseChat.tsx', 'src/components/Hub.tsx']) {
      expect(source(file), `${file} does not mount the note`).toMatch(/<PrivacyTiersOffNote\b/);
    }
  });
});
