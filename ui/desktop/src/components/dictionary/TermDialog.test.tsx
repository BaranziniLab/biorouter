import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChatGroupsState } from '../chatGroups/chatGroupsTypes';
import { getWindowId, saveChatGroups } from '../chatGroups/chatGroupsStorage';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from './copy';
import type { DictionaryAddTermPayload } from './contextMenu';
import { answer, entry, listing, settings } from './dictionaryTestKit';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  addDictionaryEntry: vi.fn(),
  editDictionaryEntry: vi.fn(),
  changeDictionaryEntryTier: vi.fn(),
  getSession: vi.fn(),
}));
vi.mock('../../api', () => api);
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
const toasts = vi.hoisted(() => ({ toastError: vi.fn() }));
vi.mock('../../toasts', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  toastError: toasts.toastError,
}));

import { userActionHeaders } from '../../utils/userAction';
import { DICTATION_MESSAGES } from '../dictation/copy';
import type { DictationStatus } from '../dictation/dictationApi';
import { refreshDictationStatus, resetDictationStoreForTests } from '../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../dictation/dictationTestKit';
import DictionaryTermDoor, { prepareAddFromMenu, startsPrivate } from './DictionaryTermDoor';
import TermDialog, { addRequestOf, editPatchOf, spokenFormsOf } from './TermDialog';
import { originAt } from './termOrigin';
import { resetDictionaryStoreForTests } from './useDictionary';

/** A status whose managed policy turns the dictionary off. */
function dictionaryOff(): DictationStatus {
  const status = speechStatus();
  status.policy = { ...status.policy, managed: true, dictionary: { allowed: false, reason: null } };
  return status;
}

/**
 * The context menu door and the term dialog (DD-68, O-6, O-36): a word picked in a private
 * chat's pane starts **Only in private chats** ticked whichever chat is current, so does one
 * in a transcript whose chat cannot be resolved, and the add names the chat it started in.
 */

/** Two panes side by side: the public chat is current (grp-1), the private one is grp-2. */
function layout(): ChatGroupsState {
  return {
    version: 1,
    layout: {
      kind: 'branch',
      dir: 'row',
      sizes: [50, 50],
      children: [
        { kind: 'leaf', groupId: 'grp-1' },
        { kind: 'leaf', groupId: 'grp-2' },
      ],
    },
    groups: {
      'grp-1': {
        groupId: 'grp-1',
        tabs: [{ tabId: 'tab-1', sessionId: 'public-chat', title: 'Public', userSetName: false }],
        activeTabId: 'tab-1',
      },
      'grp-2': {
        groupId: 'grp-2',
        tabs: [
          { tabId: 'tab-2', sessionId: 'older-chat', title: 'Older', userSetName: false },
          { tabId: 'tab-3', sessionId: 'private-chat', title: 'Private', userSetName: false },
        ],
        activeTabId: 'tab-3',
      },
    },
    activeGroupId: 'grp-1',
    seq: 3,
  };
}

const TIERS: Record<string, string> = { 'public-chat': 'public', 'private-chat': 'private' };

let fixture: HTMLElement | null = null;

function page(): { inPrivatePane: HTMLElement; inShared: HTMLElement; outside: HTMLElement } {
  fixture?.remove();
  fixture = document.createElement('div');
  document.body.appendChild(fixture);
  fixture.innerHTML = `
    <div data-chat-group-id="grp-1" data-active-group="true">
      <div data-preview-transcript=""><p id="public-word">hello</p></div>
    </div>
    <div data-chat-group-id="grp-2" data-active-group="false">
      <div data-preview-transcript=""><p id="private-word">EPAS1</p></div>
      <aside class="br-preview-panel"><span id="artifact-word">HIF2A</span></aside>
    </div>
    <main><div data-preview-transcript=""><p id="shared-word">PHD2</p></div></main>
    <nav id="outside">Settings</nav>`;
  return {
    inPrivatePane: document.getElementById('private-word')!,
    inShared: document.getElementById('shared-word')!,
    outside: document.getElementById('outside')!,
  };
}

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  toasts.toastError.mockReset();
  for (const fn of Object.values(api)) fn.mockReset();
  api.listDictionaryEntries.mockResolvedValue(listing([]));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.addDictionaryEntry.mockResolvedValue(answer(201, { entry: entry(), created: true }));
  api.getSession.mockImplementation(async ({ path }: { path: { session_id: string } }) => ({
    data: { id: path.session_id, privacy_tier: TIERS[path.session_id] },
  }));
  window.localStorage.clear();
  saveChatGroups(layout(), getWindowId());
});

afterEach(() => {
  fixture?.remove();
  fixture = null;
  delete (window as { electron?: unknown }).electron;
  resetDictationStoreForTests();
});

describe('where a word came from', () => {
  it('names the private pane’s chat while a public chat is current', () => {
    const { inPrivatePane } = page();
    expect(originAt(0, 0, { elementAt: () => inPrivatePane })).toEqual({
      kind: 'chat',
      sessionId: 'private-chat',
    });
    // The pane's artifact panel belongs to the same chat.
    const artifact = document.getElementById('artifact-word')!;
    expect(originAt(0, 0, { elementAt: () => artifact })).toEqual({
      kind: 'chat',
      sessionId: 'private-chat',
    });
  });

  it('calls a transcript outside any pane unresolved, and anything else none', () => {
    const { inShared, outside } = page();
    expect(originAt(0, 0, { elementAt: () => inShared })).toEqual({ kind: 'unresolved' });
    expect(originAt(0, 0, { elementAt: () => outside })).toEqual({ kind: 'none' });
    expect(originAt(0, 0, { elementAt: () => null })).toEqual({ kind: 'none' });
  });

  it('calls a pane’s transcript unresolved when the window’s layout cannot be read', () => {
    const { inPrivatePane } = page();
    expect(originAt(0, 0, { elementAt: () => inPrivatePane, groups: () => null })).toEqual({
      kind: 'unresolved',
    });
  });

  it('ticks Only in private chats for a private chat, an unresolved transcript or an unreadable tier', async () => {
    const tier = async (id: string) => TIERS[id] ?? null;
    expect(await startsPrivate({ kind: 'chat', sessionId: 'private-chat' }, tier)).toBe(true);
    expect(await startsPrivate({ kind: 'chat', sessionId: 'public-chat' }, tier)).toBe(false);
    expect(await startsPrivate({ kind: 'chat', sessionId: 'gone' }, tier)).toBe(true);
    expect(await startsPrivate({ kind: 'unresolved' }, tier)).toBe(true);
    expect(await startsPrivate({ kind: 'none' }, tier)).toBe(false);
  });

  it('reads the chat’s tier with the proof', async () => {
    const { inPrivatePane } = page();
    const mode = await prepareAddFromMenu(
      { text: 'EPAS1', x: 10, y: 20 },
      { elementAt: () => inPrivatePane }
    );
    expect(mode).toEqual({
      kind: 'add',
      text: 'EPAS1',
      origin: { kind: 'chat', sessionId: 'private-chat' },
      startPrivate: true,
    });
    expect(api.getSession).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'private-chat' },
        headers: { 'X-User-Action': 'proof' },
      })
    );
  });
});

describe('the add’s origin fields (O-36)', () => {
  const base = {
    written: ' EPAS1 ',
    spoken: 'epas one\n\n e pass one ',
    note: '',
    match: null,
    always: false,
  };

  it('names a resolved chat, marks an unresolved transcript, and sends neither otherwise', () => {
    expect(
      addRequestOf({ ...base, private: true, origin: { kind: 'chat', sessionId: 'private-chat' } })
    ).toEqual({
      written: 'EPAS1',
      spoken_forms: ['epas one', 'e pass one'],
      private: true,
      origin_session_id: 'private-chat',
    });
    expect(addRequestOf({ ...base, private: true, origin: { kind: 'unresolved' } })).toMatchObject({
      origin_unresolved: true,
    });
    const plain = addRequestOf({ ...base, private: false, origin: { kind: 'none' } });
    expect(plain).not.toHaveProperty('origin_session_id');
    expect(plain).not.toHaveProperty('origin_unresolved');
  });

  it('sends the match and Always use only when chosen', () => {
    expect(addRequestOf({ ...base, private: false, match: 'exact', always: true })).toMatchObject({
      match: 'exact',
      bias: 'always',
    });
  });

  it('reads one spoken form per line', () => {
    expect(spokenFormsOf(' a \n\nb\r\nc ')).toEqual(['a', 'b', 'c']);
  });
});

describe('an edit', () => {
  it('sends only what changed, and clears an emptied note', () => {
    const row = entry({
      written: 'EPAS1',
      spoken_forms: ['epas one'],
      note: 'gene',
      match: 'fuzzy',
    });
    expect(
      editPatchOf(row, {
        written: 'EPAS1',
        spoken: 'epas one',
        note: 'gene',
        match: 'fuzzy',
        always: false,
      })
    ).toEqual({});
    expect(
      editPatchOf(row, {
        written: 'EPAS1 ',
        spoken: 'epas one\ne pass one',
        note: '',
        match: 'exact',
        always: true,
      })
    ).toEqual({
      spoken_forms: ['epas one', 'e pass one'],
      note: null,
      match: 'exact',
      bias: 'always',
    });
  });

  it('asks before an untick makes a private term public, then edits and moves it', async () => {
    const row = entry({ written: 'Zeta Lab', tier: 'private', came_from_a_chat: true, rev: 4 });
    api.editDictionaryEntry.mockResolvedValue(answer(200, { entry: row }));
    api.changeDictionaryEntryTier.mockResolvedValue(
      answer(200, { entry: { ...row, tier: 'public' } })
    );
    const onClose = vi.fn();
    render(
      <TermDialog
        mode={{ kind: 'edit', entry: row }}
        authority={settings().authority}
        onClose={onClose}
      />
    );
    await userEvent.type(screen.getByTestId('dictionary-term-note'), 'our lab');
    await userEvent.click(screen.getByTestId('dictionary-term-private'));
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    expect(await screen.findByText(DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN'])).toBeInTheDocument();
    expect(api.editDictionaryEntry).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole('button', { name: DICTIONARY_LABELS.publicize.confirm })
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(api.editDictionaryEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        body: { note: 'our lab' },
        headers: expect.objectContaining({ 'If-Match': '4' }),
      })
    );
    expect(api.changeDictionaryEntryTier).toHaveBeenCalledWith(
      expect.objectContaining({ path: { id: row.id }, body: { tier: 'public' } })
    );
  });

  it('shows the daemon’s refusal under Term and stays open', async () => {
    const row = entry({ written: 'EPAS1' });
    api.editDictionaryEntry.mockResolvedValue(
      answer(409, {
        code: 'duplicate',
        message: 'That spelling is already a term here, so the edit was not made.',
      })
    );
    const onClose = vi.fn();
    render(<TermDialog mode={{ kind: 'edit', entry: row }} authority={null} onClose={onClose} />);
    const field = screen.getByTestId('dictionary-term-written');
    await userEvent.clear(field);
    await userEvent.type(field, 'UCSF');
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    expect(
      await screen.findByText('That spelling is already a term here, so the edit was not made.')
    ).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });
});

describe('an add', () => {
  it('shows MSG-SAME-LETTERS under Term before it closes', async () => {
    api.addDictionaryEntry.mockResolvedValue(
      answer(201, {
        entry: entry(),
        created: true,
        warning: 'A term with the same letters exists: epas1',
      })
    );
    const onClose = vi.fn();
    render(
      <TermDialog
        mode={{ kind: 'add', text: 'EPAS-1' }}
        authority={settings().authority}
        onClose={onClose}
      />
    );
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    expect(await screen.findByTestId('dictionary-term-same-letters')).toHaveTextContent(
      'A term with the same letters exists: epas1'
    );
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    expect(onClose).toHaveBeenCalled();
  });

  it('disables Only in private chats before the click for a caller without private standing', () => {
    const reason = 'Private terms are added on the server.';
    render(
      <TermDialog
        mode={{ kind: 'add' }}
        authority={
          settings({
            authority: { ...settings().authority, add_private: { allowed: false, reason } },
          }).authority
        }
        onClose={vi.fn()}
      />
    );
    expect(screen.getByTestId('dictionary-term-private')).toBeDisabled();
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(reason);
  });

  it('lets a caller without private standing untick a private chat’s word and add it publicly', async () => {
    const reason = 'Private terms are added on the server.';
    const onClose = vi.fn();
    render(
      <TermDialog
        mode={{
          kind: 'add',
          text: 'EPAS1',
          origin: { kind: 'chat', sessionId: 'private-chat' },
          startPrivate: true,
        }}
        authority={
          settings({
            authority: { ...settings().authority, add_private: { allowed: false, reason } },
          }).authority
        }
        onClose={onClose}
      />
    );
    // Ticked, the add cannot be made here, so Add waits with the reason before the click.
    const box = screen.getByTestId('dictionary-term-private');
    const add = screen.getByTestId('dictionary-term-save');
    expect(box).toBeChecked();
    expect(box).toBeEnabled();
    expect(add).toBeDisabled();
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(reason);
    expect(add).toHaveAttribute('aria-describedby', box.getAttribute('aria-describedby'));

    // Unticked, the warning publicize asks shows, and the word is added as a public term.
    await userEvent.click(box);
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(
      DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN']
    );
    expect(add).toBeEnabled();
    await userEvent.click(add);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(api.addDictionaryEntry.mock.calls[0][0].body).toMatchObject({
      written: 'EPAS1',
      private: false,
      origin_session_id: 'private-chat',
    });

    // Ticked again, it waits again: nothing is sent that the daemon must refuse.
    api.addDictionaryEntry.mockClear();
    await userEvent.click(box);
    expect(add).toBeDisabled();
    expect(api.addDictionaryEntry).not.toHaveBeenCalled();
  });
});

describe('the context menu door', () => {
  let deliver: ((payload: DictionaryAddTermPayload) => void) | null = null;

  beforeEach(() => {
    deliver = null;
    (window as { electron?: unknown }).electron = {
      onDictionaryAddTerm: (callback: (payload: DictionaryAddTermPayload) => void) => {
        deliver = callback;
        return () => (deliver = null);
      },
    };
  });

  async function pick(target: HTMLElement, text: string) {
    document.elementFromPoint = vi.fn(() => target);
    await act(async () => deliver!({ text, x: 120, y: 340 }));
  }

  it('opens the term dialog prefilled and ticked for a word in a private pane, and the add names that chat', async () => {
    const { inPrivatePane } = page();
    render(<DictionaryTermDoor />);
    await pick(inPrivatePane, 'EPAS1');
    expect(await screen.findByTestId('dictionary-term-written')).toHaveValue('EPAS1');
    expect(screen.getByTestId('dictionary-term-private')).toBeChecked();
    expect(document.elementFromPoint).toHaveBeenCalledWith(120, 340);
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    await waitFor(() => expect(api.addDictionaryEntry).toHaveBeenCalled());
    expect(api.addDictionaryEntry.mock.calls[0][0].body).toMatchObject({
      written: 'EPAS1',
      private: true,
      origin_session_id: 'private-chat',
    });
  });

  it('ticks it for a shared transcript and marks the add unresolved', async () => {
    const { inShared } = page();
    render(<DictionaryTermDoor />);
    await pick(inShared, 'PHD2');
    expect(await screen.findByTestId('dictionary-term-private')).toBeChecked();
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    await waitFor(() => expect(api.addDictionaryEntry).toHaveBeenCalled());
    const body = api.addDictionaryEntry.mock.calls[0][0].body;
    expect(body).toMatchObject({ written: 'PHD2', private: true, origin_unresolved: true });
    expect(body).not.toHaveProperty('origin_session_id');
  });

  it('leaves it unticked outside any transcript, and says what unticking a private chat’s word does', async () => {
    const { outside, inPrivatePane } = page();
    render(<DictionaryTermDoor />);
    await pick(outside, 'Settings');
    expect(await screen.findByTestId('dictionary-term-private')).not.toBeChecked();
    expect(screen.queryByTestId('dictionary-term-private-help')).toBeNull();

    await pick(inPrivatePane, 'EPAS1');
    await waitFor(() => expect(screen.getByTestId('dictionary-term-written')).toHaveValue('EPAS1'));
    await userEvent.click(screen.getByTestId('dictionary-term-private'));
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(
      DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN']
    );
  });

  it('says the policy’s sentence in place of the dialog under managed dictionary: false', async () => {
    resetDictationStoreForTests(fakeStatusApi(() => dictionaryOff()).api);
    const { outside } = page();
    render(<DictionaryTermDoor />);
    await pick(outside, 'EPAS1');
    await waitFor(() =>
      expect(toasts.toastError).toHaveBeenCalledWith({
        title: 'EPAS1',
        msg: DICTATION_MESSAGES['MSG-POLICY'],
      })
    );
    expect(screen.queryByTestId('dictionary-term-dialog')).toBeNull();
    expect(api.addDictionaryEntry).not.toHaveBeenCalled();
  });

  it('tells main to withhold the item once the window’s status reads the policy, and nothing before', async () => {
    let current: DictationStatus = dictionaryOff();
    resetDictationStoreForTests(fakeStatusApi(() => current).api);
    const setDictionaryOffered = vi.fn();
    Object.assign((window as unknown as { electron: object }).electron, { setDictionaryOffered });
    render(<DictionaryTermDoor />);
    // Mounting reads the store and loads nothing, so main keeps what it last heard.
    expect(setDictionaryOffered).not.toHaveBeenCalled();
    await act(async () => {
      await refreshDictationStatus();
    });
    expect(setDictionaryOffered).toHaveBeenLastCalledWith(false);
    current = speechStatus();
    await act(async () => {
      await refreshDictationStatus();
    });
    expect(setDictionaryOffered).toHaveBeenLastCalledWith(true);
    expect(setDictionaryOffered).toHaveBeenCalledTimes(2);
  });

  it('opens the dialog when the settings cannot be read, leaving the daemon to refuse', async () => {
    vi.mocked(userActionHeaders).mockRejectedValueOnce(new Error('no proof'));
    const { outside } = page();
    render(<DictionaryTermDoor />);
    await pick(outside, 'UCSF');
    expect(await screen.findByTestId('dictionary-term-written')).toHaveValue('UCSF');
    expect(api.getDictionarySettings).not.toHaveBeenCalled();
    expect(toasts.toastError).not.toHaveBeenCalled();
  });
});
