import { act, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { installResizeObserverStub } from '../test/crewTestUtils';
import type { DialogIntent } from '../state/types';
import { channelReady, currentCrew, installDaemon, renderCrew, richMessages } from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
  read: async () => '',
  getProviderModels: async () => ['fixture-model'],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * Q2-13: every Crew modal left the timeline — the log and its 27 buttons — in the accessibility
 * tree behind it, because the log carried an explicit `aria-live` and `hideOthers` keeps every
 * element that has one. A VoiceOver cursor could leave the dialog and press a message's buttons.
 * With a dialog open, the log is now inert and inside an `aria-hidden` subtree; once it closes it
 * is neither.
 */

/** The nearest ancestor (or the element) that hides it from assistive technology or input. */
function hiddenBy(element: Element, attribute: 'aria-hidden' | 'inert'): Element | null {
  for (let node: Element | null = element; node; node = node.parentElement) {
    if (attribute === 'aria-hidden' && node.getAttribute('aria-hidden') === 'true') return node;
    if (attribute === 'inert' && node.hasAttribute('inert')) return node;
  }
  return null;
}

const log = () => document.querySelector<HTMLElement>('[role="log"]') as HTMLElement;
const copyButtons = () => screen.queryAllByRole('button', { name: /^Copy text of / });

describe('a Crew modal takes the timeline out of reach behind it (Q2-13)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installDaemon({ messages: richMessages() });
  });

  it.each<[string, DialogIntent]>([
    ['Share a server path', { kind: 'share-path' }],
    ['Create team', { kind: 'create-team' }],
    ['Workspace settings', { kind: 'workspace-settings' }],
  ])(
    '%s: the log is inert and aria-hidden while it is open, and neither after',
    async (_, intent) => {
      renderCrew();
      await channelReady();
      expect(log()).not.toBeNull();
      expect(hiddenBy(log(), 'aria-hidden')).toBeNull();
      expect(hiddenBy(log(), 'inert')).toBeNull();
      const reachable = copyButtons().length;
      expect(reachable).toBeGreaterThan(0);

      act(() => currentCrew().openDialog(intent));
      await screen.findByRole('dialog');
      expect(hiddenBy(log(), 'inert')).not.toBeNull();
      expect(hiddenBy(log(), 'aria-hidden')).not.toBeNull();
      // None of the messages' buttons is in the accessibility tree behind the dialog.
      expect(copyButtons()).toHaveLength(0);

      act(() => currentCrew().closeDialog());
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(hiddenBy(log(), 'aria-hidden')).toBeNull();
      expect(hiddenBy(log(), 'inert')).toBeNull();
      expect(copyButtons()).toHaveLength(reachable);
    }
  );

  it('does the same for the sign-in dialog', async () => {
    renderCrew();
    await channelReady();
    act(() => currentCrew().openSignIn());
    await waitFor(() => expect(currentCrew().signIn.open).toBe(true));
    expect(hiddenBy(log(), 'inert')).not.toBeNull();
    expect(hiddenBy(log(), 'aria-hidden')).not.toBeNull();

    act(() => currentCrew().closeSignIn());
    await waitFor(() => expect(currentCrew().signIn.open).toBe(false));
    expect(hiddenBy(log(), 'aria-hidden')).toBeNull();
    expect(hiddenBy(log(), 'inert')).toBeNull();
  });

  it('carries no aria-live once its messages are in, so a modal’s hideOthers can hide it', async () => {
    renderCrew();
    await channelReady();
    // `role="log"` is polite by itself; an explicit attribute kept it in the tree behind modals.
    await waitFor(() => expect(log()).not.toHaveAttribute('aria-live'));
  });
});
