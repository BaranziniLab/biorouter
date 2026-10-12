import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * Task 38 (issue #56 §15.5(3)) — that the day-one notice is actually **shown**.
 *
 * ⚠ **This file exists because the notice shipped once with no caller.** Every
 * test beside it asserted that the component renders the right sentences given
 * the right props, and all of them passed while nothing in the app ever
 * constructed it: the backfill flipped 1,475 of the operator's chats and the
 * screen explaining that lived only in the source tree. §15.5 is titled "Day one
 * must be shown, not discovered". A component test cannot tell the difference;
 * this one can.
 */
const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  extensionsList: [] as Array<Record<string, unknown>>,
}));

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  listSessions: mocks.listSessions,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ extensionsList: mocks.extensionsList }),
}));

import { FirstRunPrivacyNoticeGate, FIRST_RUN_NOTICE_ACK_KEY } from './FirstRunPrivacyNoticeGate';
import {
  resetFirstRunNoticeDecisionForTests,
  startFirstRunNoticeCount,
} from './firstRunNoticeDecision';
import type { Session } from '../../api';

function row(id: string, tier: 'private' | 'public', provider: string | null, reason?: string) {
  return {
    id,
    name: id,
    description: '',
    working_dir: '/tmp',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    extension_data: {},
    message_count: 1,
    privacy_tier: tier,
    privacy_reason: reason ?? null,
    provider_name: provider,
  } as unknown as Session;
}

/** A machine the migration marked: two chats raised, one left alone. */
const BACKFILLED = [
  row('s1', 'private', 'versa_azure', 'backfill:versa_azure'),
  row('s2', 'private', 'ollama', 'backfill:ollama'),
  row('s3', 'public', 'anthropic'),
];

function served(sessions: Session[]) {
  return { data: { sessions } };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  // The count is a launch's state (FR-R4-4), so each test starts a launch.
  resetFirstRunNoticeDecisionForTests();
  mocks.extensionsList = [];
  mocks.listSessions.mockResolvedValue(served(BACKFILLED));
});

afterEach(cleanup);

describe('FirstRunPrivacyNoticeGate', () => {
  it('shows the notice on the first launch after the migration marked chats', async () => {
    render(<FirstRunPrivacyNoticeGate />);
    await screen.findByTestId('first-run-privacy-notice');
    expect((await screen.findByTestId('notice-headline')).textContent).toContain('2');
  });

  it('asks for the History population, not the one with subagents', async () => {
    render(<FirstRunPrivacyNoticeGate />);
    await screen.findByTestId('first-run-privacy-notice');
    expect(mocks.listSessions).toHaveBeenCalledWith(
      expect.objectContaining({ query: { include_subagents: false } })
    );
  });

  it('shows nothing on a machine the migration never marked', async () => {
    // A fresh install whose private chats came from the user's own turns. The
    // notice describes an upgrade that did not happen here.
    mocks.listSessions.mockResolvedValue(
      served([row('s1', 'private', 'ollama', 'turn:ollama'), row('s2', 'public', 'anthropic')])
    );
    render(<FirstRunPrivacyNoticeGate />);
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
    expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
  });

  it('shows nothing once, and stays quiet on every later launch', async () => {
    const user = userEvent.setup();
    const first = render(<FirstRunPrivacyNoticeGate />);
    await user.click(await screen.findByTestId('notice-acknowledge'));
    await waitFor(() => expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull());
    expect(localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY)).toBe('1');

    // A later launch: a fresh page load reading the same recorded acknowledgement.
    first.unmount();
    vi.clearAllMocks();
    resetFirstRunNoticeDecisionForTests();
    render(<FirstRunPrivacyNoticeGate />);
    await waitFor(() => expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull());
    // ...and it does not even ask. An acknowledged machine does no work here.
    expect(mocks.listSessions).not.toHaveBeenCalled();
  });

  it('does not burn the one-time notice on a launch where the daemon was unreachable', async () => {
    mocks.listSessions.mockRejectedValue(new Error('daemon down'));
    const first = render(<FirstRunPrivacyNoticeGate />);
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
    expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
    // Nothing was recorded, so the next launch asks again. The alternative is a
    // user who never sees the notice because their first launch after upgrading
    // raced the daemon's startup.
    expect(localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY)).toBeNull();

    first.unmount();
    resetFirstRunNoticeDecisionForTests();
    mocks.listSessions.mockResolvedValue(served(BACKFILLED));
    render(<FirstRunPrivacyNoticeGate />);
    await screen.findByTestId('first-run-privacy-notice');
  });

  // FR-R4-4: the count starts at boot (`renderer.tsx`), long before the gate mounts, and the gate
  // takes that answer instead of asking again. It used to start its own fetch on mount, a second
  // behind the first-run page on a large database.
  it('uses the count the boot started instead of fetching its own', async () => {
    startFirstRunNoticeCount();
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));
    render(<FirstRunPrivacyNoticeGate />);
    await screen.findByTestId('first-run-privacy-notice');
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
  });

  it('opens at once when the count was answered before the gate mounted', async () => {
    startFirstRunNoticeCount();
    // Let the boot's count land, with no gate on screen.
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    render(<FirstRunPrivacyNoticeGate />);
    // The first render already holds the notice: nothing is fetched, nothing is waited for.
    expect(screen.getByTestId('first-run-privacy-notice')).toBeInTheDocument();
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
  });

  // ⚠ The one assertion in this file that a render cannot make. Every test above
  // constructs the gate itself, so all of them would keep passing if `App.tsx`
  // stopped mounting it — which is exactly the state Task 38 first shipped in,
  // one level down: a correct component, fully tested, that nothing rendered.
  // The mount is part of the deliverable, so it is pinned like one.
  it('is mounted by the app, not merely exported', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    // `process.cwd()` is `ui/desktop`, where vitest.config lives.
    const app = readFileSync(resolve(process.cwd(), 'src/App.tsx'), 'utf8');
    // Anti-vacuity: a wrong path would throw, but a file that has moved and left
    // a stub behind would not. `App.tsx` mounts the sibling disclosure gate too,
    // so its absence means this is not the file being read.
    expect(app).toContain('<AppNonPrivateModelDisclosureGate />');
    expect(app).toContain("from './components/privacy/FirstRunPrivacyNoticeGate'");
    expect(app).toContain('<FirstRunPrivacyNoticeGate />');
  });

  it('names the enabled public extension that is wired to clinical data', async () => {
    // §13.5 / Open question 13. `medcp` is the operator's own machine: enabled,
    // not on the marketplace's private list, and declaring
    // `CLINICAL_RECORDS_PASSWORD`.
    mocks.extensionsList = [
      { type: 'stdio', name: 'medcp', enabled: true, env_keys: ['CLINICAL_RECORDS_PASSWORD'] },
      // Private, so the design is already working — not named.
      { type: 'stdio', name: 'ucsfomopagent', enabled: true, env_keys: ['OMOP_PASSWORD'] },
      // Public and clinical, but switched off — reaches nothing.
      { type: 'stdio', name: 'dormant', enabled: false, env_keys: ['PATIENT_TOKEN'] },
      // Enabled and public, but nothing about it points at patient data.
      { type: 'stdio', name: 'autovisualiser', enabled: true },
    ];
    render(<FirstRunPrivacyNoticeGate />);

    const paragraph = await screen.findByTestId('notice-public-clinical-extensions');
    expect(paragraph.textContent).toContain('medcp');
    expect(paragraph.textContent).not.toContain('ucsfomopagent');
    expect(paragraph.textContent).not.toContain('dormant');
    expect(paragraph.textContent).not.toContain('autovisualiser');
  });
});
