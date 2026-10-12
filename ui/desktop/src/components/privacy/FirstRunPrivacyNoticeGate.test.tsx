import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
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
import {
  BOOT_SPLASH_GONE_EVENT,
  FIRST_RUN_NOTICE_HANDOFF_WAIT_MS,
  holdFirstRunNoticeForFirstPage,
  markFirstPageDrawn,
  resetFirstRunNoticeHandoffForTests,
} from './firstRunNoticeHandoff';
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
  resetFirstRunNoticeHandoffForTests();
  delete window.__brBootSplash;
  mocks.extensionsList = [];
  mocks.listSessions.mockResolvedValue(served(BACKFILLED));
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.__brBootSplash;
});

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

  // FR-R5-1: due is not the same as ready. The boot's count can land before the splash has left and
  // before the first page has drawn, and the dialog used to open at once: over the fading wordmark,
  // on an empty ground, with the page popping in behind its blur.
  describe('opens when the launch has handed off (FR-R5-1)', () => {
    /** The boot's count, answered before the gate mounts: the order that raced. */
    async function answeredBeforeMount() {
      startFirstRunNoticeCount();
      await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 0));
    }

    it('waits for the first page the boot declared, then opens', async () => {
      holdFirstRunNoticeForFirstPage();
      await answeredBeforeMount();
      render(<FirstRunPrivacyNoticeGate />);
      // The count is in and the notice is due; the page has not drawn.
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      act(() => markFirstPageDrawn());
      expect(screen.getByTestId('first-run-privacy-notice')).toBeInTheDocument();
    });

    it('waits for the boot splash to leave, then opens', async () => {
      let gone = false;
      window.__brBootSplash = {
        dismiss: () => undefined,
        isShown: () => true,
        isGone: () => gone,
      };
      await answeredBeforeMount();
      render(<FirstRunPrivacyNoticeGate />);
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      act(() => {
        gone = true;
        window.dispatchEvent(new Event(BOOT_SPLASH_GONE_EVENT));
      });
      expect(screen.getByTestId('first-run-privacy-notice')).toBeInTheDocument();
    });

    it('opens the moment the count lands when the page and the splash are already done', async () => {
      // The other order of the race: a slow count, a page that drew long ago.
      holdFirstRunNoticeForFirstPage();
      markFirstPageDrawn();
      render(<FirstRunPrivacyNoticeGate />);
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
      await screen.findByTestId('first-run-privacy-notice');
    });

    it('opens anyway when nothing clears the hold, because the notice is a disclosure', async () => {
      holdFirstRunNoticeForFirstPage(); // and nothing ever draws
      await answeredBeforeMount();
      vi.useFakeTimers();
      render(<FirstRunPrivacyNoticeGate />);
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();

      act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_HANDOFF_WAIT_MS - 1));
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
      act(() => vi.advanceTimersByTime(1));
      expect(screen.getByTestId('first-run-privacy-notice')).toBeInTheDocument();
    });

    it('does not open for a notice that is not due, whatever has handed off', async () => {
      mocks.listSessions.mockResolvedValue(served([row('s1', 'public', 'anthropic')]));
      holdFirstRunNoticeForFirstPage();
      markFirstPageDrawn();
      render(<FirstRunPrivacyNoticeGate />);
      await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(screen.queryByTestId('first-run-privacy-notice')).toBeNull();
    });
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
