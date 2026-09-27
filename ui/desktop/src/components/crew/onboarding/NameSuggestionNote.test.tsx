import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Channel } from '../crewApi';
import { useComposerNote } from '../layout/ComposerNote';
import type { CrewController } from '../state/types';
import { nameSuggestionCopy } from './copy';
import {
  nameOfferDismissedKey,
  readJoinContext,
  resetJoinContextForTests,
  updateJoinContext,
} from './joinContext';
import { NameSuggestionNote } from './NameSuggestionNote';
import { fakeConnection, fakeSnapshot, makeCrew, renderWithCrew } from './testCrew';

const STORAGE_KEY = 'biorouter.crew.onboarding.v1';

/** Bob, verified in lab, whose display name is still his username. */
function memberCrew(overrides: Partial<CrewController> = {}) {
  return makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection(),
    connections: [fakeConnection()],
    snapshot: fakeSnapshot(),
    observedPrivacy: {
      connectionId: 'conn-1',
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    request: vi.fn(async (method: string) =>
      method === 'profile.suggest' ? { full_name: 'Bob Lee' } : {}
    ) as CrewController['request'],
    ...overrides,
  });
}

const prompt = nameSuggestionCopy.prompt('Bob Lee', 'lab');

beforeEach(() => {
  localStorage.clear();
  resetJoinContextForTests();
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe('the server-account name offer (Q3-51)', () => {
  it('reaches someone who joined before names existed: no join record is needed', async () => {
    // Nothing on this computer says this connection joined through the dialog.
    expect(readJoinContext('conn-1').suggestName).toBe(true);
    const crew = memberCrew();
    renderWithCrew(<NameSuggestionNote />, crew);
    expect(await screen.findByText(prompt)).toBeInTheDocument();
    expect(crew.mutate).not.toHaveBeenCalled();
  });

  it('is not silenced by an older build’s stored "false", written before it reached everyone', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ 'conn-1': { suggestName: false } }));
    resetJoinContextForTestsKeepingStorage();
    expect(readJoinContext('conn-1').suggestName).toBe(true);
    renderWithCrew(<NameSuggestionNote />, memberCrew());
    expect(await screen.findByText(prompt)).toBeInTheDocument();
  });

  it('shows in the composer for a member in a channel, with no join behind it', async () => {
    function Composer() {
      return <>{useComposerNote()}</>;
    }
    const channel = { id: 'c-1', name: 'general', team_id: 'team-1' } as unknown as Channel;
    renderWithCrew(<Composer />, memberCrew({ channel }));
    expect(await screen.findByText(prompt)).toBeInTheDocument();
  });

  it('remembers Dismiss per connection, in its own storage key, and asks nothing next time', async () => {
    const crew = memberCrew();
    const view = renderWithCrew(<NameSuggestionNote />, crew);
    fireEvent.click(await screen.findByRole('button', { name: nameSuggestionCopy.dismiss }));
    expect(screen.queryByText(prompt)).toBeNull();
    expect(crew.mutate).not.toHaveBeenCalled();
    expect(localStorage.getItem(nameOfferDismissedKey('conn-1'))).not.toBeNull();
    expect(nameOfferDismissedKey('conn-1')).toBe('crew:nameOffer:dismissed:conn-1');
    view.unmount();

    // A later session reads the answer back from storage.
    resetJoinContextForTestsKeepingStorage();
    const again = memberCrew();
    renderWithCrew(<NameSuggestionNote />, again);
    await act(async () => {
      await Promise.resolve();
    });
    expect(again.request).not.toHaveBeenCalled();
    expect(screen.queryByText(prompt)).toBeNull();
    // Another connection is still offered its own.
    expect(readJoinContext('conn-2').suggestName).toBe(true);
  });

  it('keeps an answer for the session when storage refuses it', async () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    const view = renderWithCrew(<NameSuggestionNote />, memberCrew());
    fireEvent.click(await screen.findByRole('button', { name: nameSuggestionCopy.dismiss }));
    expect(readJoinContext('conn-1').suggestName).toBe(false);
    view.unmount();
    const again = memberCrew();
    renderWithCrew(<NameSuggestionNote />, again);
    await act(async () => {
      await Promise.resolve();
    });
    expect(again.request).not.toHaveBeenCalled();
  });

  it('offers nothing when the server has no usable name, without recording an answer', async () => {
    const crew = memberCrew({
      request: vi.fn().mockRejectedValue(new Error('unsupported')) as CrewController['request'],
    });
    renderWithCrew(<NameSuggestionNote />, crew);
    await waitFor(() => expect(readJoinContext('conn-1').suggestName).toBe(false));
    expect(screen.queryByRole('status')).toBeNull();
    // Only this session: nothing was answered, so a later session asks again.
    expect(localStorage.getItem(nameOfferDismissedKey('conn-1'))).toBeNull();
    resetJoinContextForTestsKeepingStorage();
    expect(readJoinContext('conn-1').suggestName).toBe(true);
  });

  it('opens again for a new join or host setup of the connection', () => {
    updateJoinContext('conn-1', { suggestName: false });
    expect(readJoinContext('conn-1').suggestName).toBe(false);
    updateJoinContext('conn-1', { suggestName: true });
    expect(readJoinContext('conn-1').suggestName).toBe(true);
    expect(localStorage.getItem(nameOfferDismissedKey('conn-1'))).toBeNull();
  });
});

/**
 * A new app session over the same storage: the in-memory copy is dropped and read back. The
 * shared reset also clears storage, so the storage is put back around it.
 */
function resetJoinContextForTestsKeepingStorage() {
  const saved: Record<string, string> = {};
  for (let index = 0; index < localStorage.length; index += 1) {
    const key = localStorage.key(index);
    if (key) saved[key] = localStorage.getItem(key) ?? '';
  }
  resetJoinContextForTests();
  Object.entries(saved).forEach(([key, value]) => localStorage.setItem(key, value));
}
