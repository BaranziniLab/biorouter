import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationHistoryEntryView, DictationHistorySettings } from '../../../api';
import { DICTATION_MESSAGES } from '../../dictation/copy';
import type { DictationStatus } from '../../dictation/dictationApi';
import { resetDictationStoreForTests } from '../../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from '../copy';
import { answer, entry, listing, scan, settings } from '../dictionaryTestKit';
import { dictation, historyAnswer, historyPage, historySettings } from './historyTestKit';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  getDictionarySuggestionScan: vi.fn(),
  addDictionaryEntry: vi.fn(),
  getSession: vi.fn(),
  listDictationHistory: vi.fn(),
  getDictationHistorySettings: vi.fn(),
  changeDictationHistorySettings: vi.fn(),
  changeDictationHistoryTier: vi.fn(),
  deleteDictationHistoryEntry: vi.fn(),
  clearDictationHistory: vi.fn(),
}));
vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../api')>()),
  ...api,
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
vi.mock('../../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
const toasts = vi.hoisted(() => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));
vi.mock('../../../toasts', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  toastError: toasts.toastError,
  toastSuccess: toasts.toastSuccess,
}));

import DictionaryView from '../DictionaryView';
import { DICTIONARY_TAB_STORAGE_KEY } from '../navigation';
import { resetDictionaryStoreForTests } from '../useDictionary';
import { historyRereadForTests, resetHistoryStoreForTests } from '../useDictationHistory';
import { resetSuggestionScanForTests } from '../useSuggestionScan';

type Call = { query?: { q?: string; cursor?: string }; headers?: Record<string, string> };

let rows: DictationHistoryEntryView[] = [];
let currentSettings: DictationHistorySettings = historySettings();

function useStatus(status: () => DictationStatus) {
  resetDictationStoreForTests(fakeStatusApi(status).api);
}

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetHistoryStoreForTests();
  resetSuggestionScanForTests();
  useStatus(() => speechStatus());
  for (const fn of Object.values(api)) fn.mockReset();
  toasts.toastError.mockReset();
  toasts.toastSuccess.mockReset();
  window.localStorage.clear();
  rows = [];
  currentSettings = historySettings();
  api.listDictionaryEntries.mockImplementation(async () => listing([]));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.getDictionarySuggestionScan.mockResolvedValue(answer(200, scan()));
  api.listDictationHistory.mockImplementation(async ({ query }: Call) => {
    const q = query?.q?.toLowerCase() ?? '';
    const found = rows.filter((row) => row.text.toLowerCase().includes(q));
    return historyPage(found, { total: found.length });
  });
  api.getDictationHistorySettings.mockImplementation(async () =>
    historyAnswer(200, currentSettings)
  );
  api.getSession.mockResolvedValue({ data: { privacy_tier: 'public' } });
});

afterEach(() => {
  resetDictationStoreForTests();
});

function Where() {
  const location = useLocation();
  return (
    <span data-testid="where">
      {location.pathname}
      {location.search}|{JSON.stringify(location.state ?? null)}
    </span>
  );
}

function renderPage(at = '/dictionary?tab=history') {
  return render(
    <MemoryRouter initialEntries={[at]}>
      <Routes>
        <Route path="/dictionary" element={<DictionaryView />} />
        <Route path="*" element={<Where />} />
      </Routes>
    </MemoryRouter>
  );
}

const headings = () =>
  [...document.querySelectorAll('[data-history-section] h2')].map((h) => h.textContent);
const rowOf = (text: string) => screen.getByText(text).closest('[data-history-row]') as HTMLElement;

async function openMenu(text: string) {
  await userEvent.click(within(rowOf(text)).getByTestId('history-row-menu'));
}

describe('the tab', () => {
  it('opens from #/dictionary?tab=history, and is remembered once chosen', async () => {
    renderPage();
    expect(await screen.findByTestId('history-tab')).toBeInTheDocument();
    expect(screen.getByTestId('dictionary-tab-history')).toHaveAttribute('data-state', 'active');

    await userEvent.click(screen.getByTestId('dictionary-tab-terms'));
    expect(window.localStorage.getItem(DICTIONARY_TAB_STORAGE_KEY)).toBe('terms');
    expect(await screen.findByTestId('dictionary-groups')).toBeInTheDocument();
    expect(screen.queryByTestId('history-tab')).toBeNull();
  });

  it('opens on the remembered tab, and on Terms when nothing was stored', async () => {
    window.localStorage.setItem(DICTIONARY_TAB_STORAGE_KEY, 'history');
    const first = renderPage('/dictionary');
    expect(await screen.findByTestId('history-tab')).toBeInTheDocument();
    first.unmount();
    window.localStorage.clear();
    renderPage('/dictionary');
    expect(await screen.findByTestId('dictionary-groups')).toBeInTheDocument();
    expect(screen.queryByTestId('history-tab')).toBeNull();
    expect(api.listDictationHistory).toHaveBeenCalledTimes(1);
  });

  it('opens on Terms when storage is blocked', async () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    try {
      renderPage('/dictionary');
      expect(await screen.findByTestId('dictionary-groups')).toBeInTheDocument();
      expect(screen.getByTestId('dictionary-tab-terms')).toHaveAttribute('data-state', 'active');
    } finally {
      getItem.mockRestore();
    }
  });

  it('has its own band: the search, the settings link and Clear history…, and no primary', async () => {
    rows = [dictation({ text: 'EPAS1 binds HIF' })];
    renderPage();
    await screen.findByText('EPAS1 binds HIF');
    expect(screen.getByRole('searchbox', { name: /Search dictations/ })).toBeInTheDocument();
    expect(screen.queryByTestId('dictionary-add')).toBeNull();
    expect(screen.queryByTestId('dictionary-export')).toBeNull();
    // The count is the dictations the caller may list.
    expect(screen.getByRole('banner')).toHaveTextContent('1');
    await userEvent.click(screen.getByTestId('history-settings-link'));
    expect(screen.getByTestId('where')).toHaveTextContent(
      JSON.stringify({ section: 'dictation-history' })
    );
  });
});

describe('the rows', () => {
  it('groups dictations under the chat lists’ date buckets', async () => {
    const now = Date.now();
    rows = [
      dictation({ text: 'today one', minutesAgo: 1, now }),
      dictation({ text: 'last week', minutesAgo: 60 * 24 * 4, now }),
      dictation({ text: 'a while ago', minutesAgo: 60 * 24 * 20, now }),
    ];
    renderPage();
    await screen.findByText('today one');
    expect(headings()).toEqual(['Today', 'Previous 7 days', 'Previous 30 days']);
  });

  it('badges only private rows with the lock, and names each glyph', async () => {
    rows = [
      dictation({ text: 'private one', tier: 'private' }),
      dictation({ text: 'public one', tier: 'public' }),
    ];
    renderPage();
    await screen.findByText('private one');
    expect(
      within(rowOf('private one')).getByRole('img', { name: 'Private dictation' })
    ).toBeTruthy();
    expect(within(rowOf('public one')).getByRole('img', { name: 'Public dictation' })).toBeTruthy();
    expect(within(rowOf('public one')).queryByTestId('history-row-private')).toBeNull();
  });

  it('expands a row in place on a click, naming the model, and collapses it on another', async () => {
    rows = [dictation({ text: 'Query SPOKE for EPAS1', model_name: 'Large v3 Turbo' })];
    renderPage();
    const body = within(await waitFor(() => rowOf('Query SPOKE for EPAS1'))).getByTestId(
      'history-row-body'
    );
    expect(body).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText(/Large v3 Turbo/)).toBeNull();
    await userEvent.click(body);
    expect(body).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('history-row-expanded-line')).toHaveTextContent(
      /^Private · .* · Chat · 14 s · Large v3 Turbo$/
    );
    fireEvent.keyDown(body, { key: 'Enter' });
    expect(body).toHaveAttribute('aria-expanded', 'false');
  });

  it('copies the whole text from the hover action', async () => {
    const writeText = vi.fn(async () => undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    rows = [dictation({ text: 'Copy me whole' })];
    renderPage();
    await screen.findByText('Copy me whole');
    await userEvent.click(within(rowOf('Copy me whole')).getByTestId('history-row-copy'));
    expect(writeText).toHaveBeenCalledWith('Copy me whole');
    expect(toasts.toastSuccess).toHaveBeenCalledWith({ title: DICTIONARY_LABELS.history.copied });
  });

  it('offers Open the chat only for a dictation whose chat the daemon names', async () => {
    rows = [
      dictation({ text: 'in a chat', session_id: '20261126_3' }),
      dictation({ text: 'on the clipboard', surface: 'clipboard' }),
    ];
    renderPage();
    await screen.findByText('in a chat');
    await openMenu('on the clipboard');
    expect(screen.queryByTestId('history-open-chat')).toBeNull();
    await userEvent.keyboard('{Escape}');
    await openMenu('in a chat');
    await userEvent.click(screen.getByTestId('history-open-chat'));
    expect(screen.getByTestId('where')).toHaveTextContent('/pair?resumeSessionId=20261126_3');
  });

  it('asks the warning that names private terms before Make public…', async () => {
    rows = [dictation({ text: 'with terms', tier: 'private', private_terms: true })];
    api.changeDictationHistoryTier.mockImplementation(async () =>
      historyAnswer(200, { entry: { ...rows[0], tier: 'public' } })
    );
    renderPage();
    await screen.findByText('with terms');
    await openMenu('with terms');
    await userEvent.click(screen.getByTestId('history-make-public'));
    expect(
      await screen.findByText(DICTIONARY_MESSAGES['MSG-HISTORY-PUBLICIZE-WARN-TERMS'])
    ).toBeInTheDocument();
    expect(api.changeDictationHistoryTier).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole('button', { name: DICTIONARY_LABELS.history.publicize.confirm })
    );
    await waitFor(() => expect(api.changeDictationHistoryTier).toHaveBeenCalled());
    expect(api.changeDictationHistoryTier.mock.calls[0][0]).toMatchObject({
      body: { tier: 'public' },
      headers: { 'X-User-Action': 'proof' },
    });
    await historyRereadForTests();
  });

  it('makes a dictation private at once, with no question', async () => {
    rows = [dictation({ text: 'now public', tier: 'public' })];
    api.changeDictationHistoryTier.mockImplementation(async () =>
      historyAnswer(200, { entry: { ...rows[0], tier: 'private' } })
    );
    renderPage();
    await screen.findByText('now public');
    await openMenu('now public');
    await userEvent.click(screen.getByTestId('history-make-private'));
    await waitFor(() =>
      expect(api.changeDictationHistoryTier.mock.calls[0][0]).toMatchObject({
        body: { tier: 'private' },
      })
    );
    expect(screen.queryByRole('dialog')).toBeNull();
    await historyRereadForTests();
  });

  it('disables a tier change before the click with the host’s sentence for this row', async () => {
    rows = [dictation({ text: 'on the host', tier: 'private' })];
    currentSettings = historySettings({
      authority: {
        ...historySettings().authority,
        tier_change: {
          allowed: false,
          reason: DICTIONARY_MESSAGES['MSG-HISTORY-HOST-TIER'].replace('<verb>', 'publicize'),
        },
      },
    });
    renderPage();
    await screen.findByText('on the host');
    await openMenu('on the host');
    expect(screen.getByTestId('history-make-public')).toHaveAttribute('data-disabled');
    expect(
      screen.getByText(
        `Do this on the server: \`biorouter dictation history publicize ${rows[0].id}\` (as the service user).`
      )
    ).toBeInTheDocument();
  });

  it('deletes one dictation after asking, and says Deleted in place while the pointer holds the list', async () => {
    rows = [dictation({ text: 'keep me' }), dictation({ text: 'delete me' })];
    api.deleteDictationHistoryEntry.mockImplementation(async () => {
      rows = rows.filter((row) => row.text !== 'delete me');
      return historyAnswer(204, null);
    });
    renderPage();
    await screen.findByText('delete me');
    fireEvent.pointerEnter(screen.getByTestId('history-groups'));
    await openMenu('delete me');
    await userEvent.click(screen.getByTestId('history-delete'));
    expect(await screen.findByText(DICTIONARY_MESSAGES['MSG-HISTORY-DELETE'])).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole('button', { name: DICTIONARY_LABELS.history.deleteOne.confirm })
    );
    await waitFor(() => expect(api.deleteDictationHistoryEntry).toHaveBeenCalled());
    expect(
      await within(rowOf('delete me')).findByText(DICTIONARY_LABELS.history.settled.deleted)
    ).toBeInTheDocument();
    fireEvent.pointerLeave(screen.getByTestId('history-groups'));
    await waitFor(() => expect(screen.queryByText('delete me')).toBeNull());
    await historyRereadForTests();
  });
});

describe('the right click', () => {
  /** Select `text` inside the row's paragraph, as a person dragging over it would. */
  function select(text: string, within: string) {
    const paragraph = screen.getByText(within);
    const node = paragraph.firstChild!;
    const start = node.textContent!.indexOf(text);
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, start + text.length);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
  }

  afterEach(() => window.getSelection()?.removeAllRanges());

  it('opens the row’s menu, and lets the app’s own menu win on text selected inside it', async () => {
    rows = [dictation({ text: 'Spell EGLN1 right' })];
    renderPage();
    await screen.findByText('Spell EGLN1 right');
    const body = within(rowOf('Spell EGLN1 right')).getByTestId('history-row-body');

    select('EGLN1', 'Spell EGLN1 right');
    const passed = fireEvent.contextMenu(body);
    // Not prevented: the native menu, with **Add to Biorouter dictionary…**, opens.
    expect(passed).toBe(true);
    expect(screen.queryByTestId('history-copy-text')).toBeNull();

    window.getSelection()!.removeAllRanges();
    fireEvent.contextMenu(body);
    expect(await screen.findByTestId('history-copy-text')).toBeInTheDocument();
  });

  it('prefills Add to dictionary… with the text selected inside the dictation', async () => {
    rows = [dictation({ text: 'Spell EGLN1 right', tier: 'public' })];
    renderPage();
    await screen.findByText('Spell EGLN1 right');
    select('EGLN1', 'Spell EGLN1 right');
    const menu = within(rowOf('Spell EGLN1 right')).getByTestId('history-row-menu');
    await userEvent.click(menu);
    await userEvent.click(screen.getByTestId('history-add-to-dictionary'));
    expect(await screen.findByTestId('dictionary-term-written')).toHaveValue('EGLN1');
    expect(screen.getByTestId('dictionary-term-private')).not.toBeChecked();
  });
});

describe('Add to dictionary…', () => {
  it('opens the term dialog ticked for a private dictation, naming its chat', async () => {
    rows = [dictation({ text: 'Spell EPAS1 right', tier: 'private', session_id: 'chat-1' })];
    api.addDictionaryEntry.mockResolvedValue(answer(201, { entry: entry(), created: true }));
    renderPage();
    await screen.findByText('Spell EPAS1 right');
    await openMenu('Spell EPAS1 right');
    await userEvent.click(screen.getByTestId('history-add-to-dictionary'));
    const written = await screen.findByTestId('dictionary-term-written');
    expect(written).toHaveValue('');
    expect(screen.getByTestId('dictionary-term-private')).toBeChecked();
    await userEvent.type(written, 'EPAS1');
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    await waitFor(() => expect(api.addDictionaryEntry).toHaveBeenCalled());
    expect(api.addDictionaryEntry.mock.calls[0][0].body).toMatchObject({
      written: 'EPAS1',
      private: true,
      origin_session_id: 'chat-1',
    });
  });

  it('warns in the history’s words when a private dictation’s word is unticked', async () => {
    rows = [dictation({ text: 'from the clipboard', tier: 'private', surface: 'clipboard' })];
    renderPage();
    await screen.findByText('from the clipboard');
    await openMenu('from the clipboard');
    await userEvent.click(screen.getByTestId('history-add-to-dictionary'));
    await userEvent.click(await screen.findByTestId('dictionary-term-private'));
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(
      DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN-HISTORY']
    );
  });

  it('is not offered where the caller may not write the dictionary', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: {
            ...settings().authority,
            write: { allowed: false, reason: 'Read only here.' },
          },
        })
      )
    );
    rows = [dictation({ text: 'read only' })];
    renderPage();
    await screen.findByText('read only');
    await waitFor(() => expect(api.getDictionarySettings).toHaveBeenCalled());
    await act(async () => undefined);
    await openMenu('read only');
    expect(screen.queryByTestId('history-add-to-dictionary')).toBeNull();
  });
});

describe('the search', () => {
  it('asks the daemon 200 ms after the last keystroke, and says when nothing matches', async () => {
    rows = [dictation({ text: 'EPAS1 binds HIF' }), dictation({ text: 'Unrelated' })];
    renderPage();
    await screen.findByText('EPAS1 binds HIF');
    api.listDictationHistory.mockClear();
    const field = screen.getByRole('searchbox', { name: /Search dictations/ });
    for (const typed of ['E', 'EP', 'EPA', 'EPAS1'])
      fireEvent.change(field, { target: { value: typed } });
    expect(api.listDictationHistory).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByText('Unrelated')).toBeNull());
    const asked = api.listDictationHistory.mock.calls.map((call) => (call[0] as Call).query?.q);
    expect(asked).toEqual(['EPAS1']);

    fireEvent.change(field, { target: { value: 'zebrafish' } });
    expect(await screen.findByText(DICTIONARY_LABELS.history.noMatches)).toBeInTheDocument();
    expect(screen.getByText(DICTIONARY_MESSAGES['MSG-HISTORY-NO-MATCH'])).toBeInTheDocument();
  });
});

describe('Clear history…', () => {
  it('asks for the typed word, then clears through the newest dictation shown', async () => {
    rows = [
      dictation({ text: 'newest', minutesAgo: 1 }),
      dictation({ text: 'older', minutesAgo: 9 }),
    ];
    const newestAt = rows[0].created_at;
    api.clearDictationHistory.mockImplementation(async () => {
      rows = [];
      return historyAnswer(200, { deleted: 2 });
    });
    renderPage();
    await screen.findByText('newest');
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.history.pageMenu }));
    await userEvent.click(screen.getByTestId('history-clear'));
    expect(
      await screen.findByText(DICTIONARY_MESSAGES['MSG-HISTORY-CLEAR'].replace('<n>', '2'))
    ).toBeInTheDocument();
    const confirm = screen.getByRole('button', {
      name: DICTIONARY_LABELS.history.clearDialog.confirm,
    });
    expect(confirm).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText(DICTIONARY_LABELS.history.clearDialog.field),
      'delete'
    );
    await userEvent.click(confirm);
    await waitFor(() => expect(api.clearDictationHistory).toHaveBeenCalled());
    expect(api.clearDictationHistory.mock.calls[0][0]).toMatchObject({
      body: { through: newestAt },
      headers: { 'X-User-Action': 'proof' },
    });
    expect(await screen.findByText(DICTIONARY_LABELS.history.empty)).toBeInTheDocument();
  });

  it('is disabled with nothing to clear', async () => {
    renderPage();
    await screen.findByText(DICTIONARY_LABELS.history.empty);
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.history.pageMenu }));
    expect(screen.getByTestId('history-clear')).toHaveAttribute('data-disabled');
  });
});

describe('every empty and special state', () => {
  it('shows three skeleton rows only while the first read is out', async () => {
    let release: (() => void) | null = null;
    api.listDictationHistory.mockImplementationOnce(async () => {
      await new Promise<void>((done) => (release = done));
      return historyPage([]);
    });
    renderPage();
    const skeleton = await screen.findByTestId('history-skeleton');
    expect(skeleton.querySelectorAll('.biorouter-list-row')).toHaveLength(3);
    await act(async () => release!());
    await waitFor(() => expect(screen.queryByTestId('history-skeleton')).toBeNull());
  });

  it('says No dictations yet while the history is on and empty', async () => {
    renderPage();
    expect(await screen.findByText(DICTIONARY_LABELS.history.empty)).toBeInTheDocument();
    expect(screen.getByText(DICTIONARY_MESSAGES['MSG-HISTORY-EMPTY'])).toBeInTheDocument();
  });

  it('says the history is off, and Turn on writes the switch with the proof', async () => {
    currentSettings = historySettings({ keep_history: false });
    api.changeDictationHistorySettings.mockImplementation(async () => {
      currentSettings = historySettings({ keep_history: true });
      return historyAnswer(200, currentSettings);
    });
    renderPage();
    const empty = await screen.findByTestId('history-empty-off');
    expect(within(empty).getByText(DICTIONARY_LABELS.history.off)).toBeInTheDocument();
    await userEvent.click(within(empty).getByTestId('history-turn-on'));
    await waitFor(() => expect(api.changeDictationHistorySettings).toHaveBeenCalled());
    expect(api.changeDictationHistorySettings.mock.calls[0][0]).toMatchObject({
      body: { keep_history: true },
      headers: { 'X-User-Action': 'proof' },
    });
    expect(await screen.findByText(DICTIONARY_LABELS.history.empty)).toBeInTheDocument();
  });

  it('disables Turn on before the click where a raise is refused, saying why', async () => {
    currentSettings = historySettings({
      keep_history: false,
      authority: {
        ...historySettings().authority,
        raise_settings: {
          allowed: false,
          reason: DICTIONARY_MESSAGES['MSG-HISTORY-HOST-SETTINGS'],
        },
      },
    });
    renderPage();
    expect(await screen.findByTestId('history-turn-on')).toBeDisabled();
    expect(screen.getByTestId('history-turn-on-reason')).toHaveTextContent('settings --keep on');
  });

  it('keeps the list with one note while the history is off', async () => {
    currentSettings = historySettings({ keep_history: false });
    rows = [dictation({ text: 'kept before' })];
    renderPage();
    await screen.findByText('kept before');
    expect(screen.getByTestId('history-off-note')).toHaveTextContent(
      DICTIONARY_MESSAGES['MSG-HISTORY-OFF']
    );
  });

  it('says where private dictations are shown to a caller that cannot list them', async () => {
    const reason = DICTIONARY_MESSAGES['MSG-HISTORY-PRIVATE-ELSEWHERE'].replace(
      '<host>',
      'lab-host'
    );
    currentSettings = historySettings({
      authority: { ...historySettings().authority, list_private: { allowed: false, reason } },
    });
    renderPage();
    expect(await screen.findByTestId('history-private-elsewhere')).toHaveTextContent(reason);
  });

  it('keeps the list under managed history: false, with delete allowed and tier changes refused', async () => {
    const managed = speechStatus();
    (managed.policy as unknown as Record<string, unknown>).history = {
      allowed: false,
      reason: 'Your organization turned dictation history off.',
    };
    useStatus(() => managed);
    rows = [dictation({ text: 'managed one', tier: 'private' })];
    renderPage();
    await screen.findByText('managed one');
    expect(await screen.findByTestId('history-policy')).toHaveTextContent(
      'Your organization turned dictation history off.'
    );
    expect(screen.queryByTestId('history-settings-link')).toBeNull();
    await openMenu('managed one');
    expect(screen.getByTestId('history-make-public')).toHaveAttribute('data-disabled');
    expect(screen.getByTestId('history-delete')).not.toHaveAttribute('data-disabled');
  });

  it('says the history file could not be read', async () => {
    const message = 'The dictation history file at /h could not be read, so nothing was changed.';
    api.listDictationHistory.mockResolvedValue(historyAnswer(503, { code: 'unreadable', message }));
    renderPage();
    expect(await screen.findByTestId('history-notice')).toHaveTextContent(message);
    expect(screen.queryByText(DICTIONARY_LABELS.history.empty)).toBeNull();
  });

  it('says a backend from before the history is too old', async () => {
    api.listDictationHistory.mockResolvedValue(historyAnswer(404, {}));
    renderPage();
    const notice = await screen.findByTestId('history-notice');
    expect(notice.textContent).toContain(DICTATION_MESSAGES['MSG-REMOTE-OLD'].slice(0, 20));
    expect(screen.queryByTestId('history-settings-link')).toBeNull();
  });
});

describe('every call', () => {
  it('sends the proof', async () => {
    rows = [dictation({ text: 'proof one', tier: 'public' })];
    api.changeDictationHistoryTier.mockImplementation(async () =>
      historyAnswer(200, { entry: { ...rows[0], tier: 'private' } })
    );
    renderPage();
    await screen.findByText('proof one');
    fireEvent.change(screen.getByRole('searchbox', { name: /Search dictations/ }), {
      target: { value: 'proof' },
    });
    await waitFor(() =>
      expect(api.listDictationHistory.mock.calls.some((c) => (c[0] as Call).query?.q)).toBe(true)
    );
    await openMenu('proof one');
    await userEvent.click(screen.getByTestId('history-make-private'));
    await waitFor(() => expect(api.changeDictationHistoryTier).toHaveBeenCalled());
    await historyRereadForTests();
    const historyCalls = [
      api.listDictationHistory,
      api.getDictationHistorySettings,
      api.changeDictationHistoryTier,
    ].flatMap((fn) => fn.mock.calls.map((call) => call[0] as Call));
    expect(historyCalls.length).toBeGreaterThan(3);
    for (const call of historyCalls) {
      expect(call.headers).toMatchObject({ 'X-User-Action': 'proof' });
    }
  });
});
