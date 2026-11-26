import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictionaryEntryView } from '../../api';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { DICTATION_MESSAGES } from '../dictation/copy';
import { refreshDictationStatus, resetDictationStoreForTests } from '../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from './copy';
import { answer, entry, listing, scan, settings } from './dictionaryTestKit';
import { groupEntries, lineTwo, privateLine, reviewRows, usesMeta } from './rows';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  changeDictionarySettings: vi.fn(),
  activateDictionary: vi.fn(),
  addDictionaryEntry: vi.fn(),
  editDictionaryEntry: vi.fn(),
  changeDictionaryEntryTier: vi.fn(),
  deleteDictionaryEntry: vi.fn(),
  deleteDictionaryEntries: vi.fn(),
  acceptDictionarySuggestions: vi.fn(),
  dismissDictionarySuggestions: vi.fn(),
  keepDictionaryEntries: vi.fn(),
  importDictionary: vi.fn(),
  exportDictionary: vi.fn(),
  getDictionarySuggestionScan: vi.fn(),
  startDictionarySuggestionScan: vi.fn(),
  stopDictionarySuggestionScan: vi.fn(),
  listDictationHistory: vi.fn(),
  getDictationHistorySettings: vi.fn(),
}));
vi.mock('../../api', () => api);
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
// Radix's ScrollArea needs a ResizeObserver jsdom does not have, and the page only needs the
// children (the stand-in the Skills, Workflows and Scheduler tests use).
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

import DictionaryView from './DictionaryView';
import { resetDictionaryStoreForTests } from './useDictionary';
import { resetHistoryStoreForTests } from './useDictationHistory';
import { resetSuggestionScanForTests } from './useSuggestionScan';

let rows: DictionaryEntryView[] = [];

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetHistoryStoreForTests();
  resetSuggestionScanForTests();
  window.localStorage.clear();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  for (const fn of Object.values(api)) fn.mockReset();
  rows = [];
  api.listDictionaryEntries.mockImplementation(async () => listing(rows));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.changeDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.activateDictionary.mockResolvedValue(answer(200, settings({ suggest_from_chats: true })));
  api.acceptDictionarySuggestions.mockResolvedValue(answer(200, { accepted: 1 }));
  api.dismissDictionarySuggestions.mockResolvedValue(answer(200, { dismissed: 1 }));
  api.keepDictionaryEntries.mockResolvedValue(answer(200, { kept: 1 }));
  api.deleteDictionaryEntries.mockResolvedValue(answer(200, { deleted: 1 }));
  api.getDictionarySuggestionScan.mockResolvedValue(answer(200, scan()));
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

function renderPage() {
  return render(
    <MemoryRouter initialEntries={['/dictionary']}>
      <DictionaryView />
    </MemoryRouter>
  );
}

const sections = () =>
  [...document.querySelectorAll('[data-dictionary-section]')].map((section) =>
    section.getAttribute('data-dictionary-section')
  );

const UCSF = { id: 'ucsf', name: 'UCSF' };
const STANFORD = { id: 'stanford', name: 'Stanford' };

describe('the groups', () => {
  const mixed = () => [
    entry({ written: 'zeta', source: 'user' }),
    entry({ written: 'Alpha', source: 'user' }),
    entry({ written: 'Suggested one', status: 'suggested', source: 'auto' }),
    entry({ written: 'Agent new', source: 'agent', reviewed: false }),
    entry({ written: 'Agent kept', source: 'agent', reviewed: true }),
    entry({ written: 'Learned', source: 'correction' }),
    entry({ written: 'Found', source: 'auto' }),
    entry({ written: 'From a file', source: 'import' }),
  ];

  it('orders Suggested, Added by the agent, Added by you, From your chats, Imported', () => {
    const groups = groupEntries(mixed());
    expect(groups.map((group) => group.key)).toEqual([
      'suggested',
      'agent',
      'user',
      'chats',
      'imported',
    ]);
    expect(groups.find((g) => g.key === 'chats')!.entries.map((e) => e.written)).toEqual([
      'Agent kept',
      'Found',
      'Learned',
    ]);
    // Alphabetical, ignoring case, so a row keeps its place when its usage changes.
    expect(groups.find((g) => g.key === 'user')!.entries.map((e) => e.written)).toEqual([
      'Alpha',
      'zeta',
    ]);
  });

  it('always shows Added by you, and the others only when present', () => {
    expect(groupEntries([entry({ status: 'suggested' })]).map((g) => g.key)).toEqual([
      'suggested',
      'user',
    ]);
    expect(groupEntries([])).toEqual([]);
  });

  it('folds every group into one Matches list for a filter over written, spoken forms and note', () => {
    const list = [
      entry({ written: 'EPAS1', spoken_forms: ['epas one'] }),
      entry({ written: 'UCSF', note: 'the campus', status: 'suggested' }),
      entry({ written: 'Other' }),
    ];
    expect(groupEntries(list, 'EPAS').map((g) => g.entries.map((e) => e.written))).toEqual([
      ['EPAS1'],
    ]);
    expect(groupEntries(list, 'campus')[0]).toMatchObject({ key: 'matches' });
    expect(groupEntries(list, 'epas ONE')[0].entries).toHaveLength(1);
  });

  it('reviews the suggestions and the agent’s rows not yet kept', () => {
    expect(
      reviewRows(mixed())
        .map((e) => e.written)
        .sort()
    ).toEqual(['Agent new', 'Suggested one']);
  });

  it('renders the groups with their rows, and Review all… on Suggested', async () => {
    rows = mixed();
    renderPage();
    await waitFor(() =>
      expect(sections()).toEqual(['suggested', 'agent', 'user', 'chats', 'imported'])
    );
    expect(screen.getByTestId('dictionary-review-all')).toHaveTextContent(
      DICTIONARY_LABELS.reviewAll
    );
    expect(screen.getByRole('heading', { name: /Added by you 2/ })).toBeInTheDocument();
    expect(screen.queryByText(DICTIONARY_LABELS.userGroupEmpty)).toBeNull();
  });

  it('says so under an empty Added by you, as in a dictionary the agent started', async () => {
    rows = [entry({ written: 'EPAS1', source: 'agent', reviewed: false })];
    renderPage();
    await waitFor(() => expect(sections()).toEqual(['agent', 'user']));
    const user = document.querySelector('[data-dictionary-section="user"]') as HTMLElement;
    expect(within(user).getByRole('heading', { name: /Added by you 0/ })).toBeInTheDocument();
    expect(within(user).getByText(DICTIONARY_LABELS.userGroupEmpty)).toBeInTheDocument();
    expect(user.querySelector('[data-dictionary-rows]')).toBeNull();
    const agent = document.querySelector('[data-dictionary-section="agent"]') as HTMLElement;
    expect(within(agent).queryByText(DICTIONARY_LABELS.userGroupEmpty)).toBeNull();
  });
});

describe('a row’s second line', () => {
  it('says how the term is said, else its note, else where it came from', () => {
    expect(lineTwo(entry({ spoken_forms: ['epas one', 'e pass one'] }))).toBe(
      'Said as epas one, e pass one'
    );
    expect(lineTwo(entry({ note: 'hypoxia gene' }))).toBe('hypoxia gene');
    expect(lineTwo(entry({ source: 'import' }))).toBe('Imported');
    expect(lineTwo(entry({ source: 'correction' }))).toBe('Learned from your corrections');
  });

  it('starts a private row with where it is used, for each kind of affiliation', () => {
    const p = (affiliation: DictionaryEntryView['affiliation']) =>
      entry({ tier: 'private', affiliation, spoken_forms: ['epas one'] });
    expect(lineTwo(p(null))).toBe(`${DICTIONARY_MESSAGES['MSG-ROW-PRIVATE']} · Said as epas one`);
    expect(privateLine(p({ institutions: [UCSF] }))).toBe(
      'Used only in private chats on UCSF models and on local models'
    );
    // The published name, else the id.
    expect(privateLine(p({ institutions: [{ id: 'mayo', name: null }] }))).toBe(
      'Used only in private chats on mayo models and on local models'
    );
    expect(privateLine(p({ institutions: [UCSF, STANFORD] }))).toBe(
      'Tied to UCSF and Stanford; used only in private chats on local models'
    );
    expect(privateLine(p({ institutions: [] }))).toBe(DICTIONARY_MESSAGES['MSG-ROW-PRIVATE-LOCAL']);
    expect(privateLine(entry())).toBeNull();
  });

  it('shows usage only when the answer carries it, and reads an absent last_used_at as never', () => {
    expect(usesMeta(entry({ uses: 12, last_used_at: '2026-11-04T17:02:11Z' }))).toBe('12 uses');
    expect(usesMeta(entry({ uses: 1, last_used_at: '2026-11-04T17:02:11Z' }))).toBe('1 use');
    expect(usesMeta(entry({ uses: 0 }))).toBe(DICTIONARY_LABELS.notUsed);
    // No private standing: no usage at all.
    expect(usesMeta(entry({ uses: undefined, corrected_to: undefined }))).toBeNull();
    expect(usesMeta(entry({ status: 'suggested', uses: 0 }))).toBeNull();
  });

  it('draws the lock badge and the private line on the page', async () => {
    rows = [
      entry({
        written: 'Zeta Lab',
        tier: 'private',
        affiliation: { institutions: [UCSF, STANFORD] },
      }),
      entry({ written: 'UCSF' }),
    ];
    renderPage();
    const privateRow = await screen.findByText('Zeta Lab');
    const row = privateRow.closest('[data-dictionary-row]') as HTMLElement;
    expect(within(row).getByTestId('dictionary-row-private')).toHaveAttribute(
      'aria-label',
      DICTIONARY_LABELS.privateTerm
    );
    expect(within(row).getByTestId('dictionary-row-line-two')).toHaveTextContent(
      'Tied to UCSF and Stanford; used only in private chats on local models · Added by you'
    );
    const publicRow = screen.getByText('UCSF').closest('[data-dictionary-row]') as HTMLElement;
    expect(within(publicRow).queryByTestId('dictionary-row-private')).toBeNull();
  });
});

describe('the page', () => {
  it('keeps browser writes closed when a settings response has no authority', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    rows = [entry({ written: 'EPAS1' }), entry({ written: 'Maybe', status: 'suggested' })];
    api.getDictionarySettings.mockResolvedValue(answer(200, { ...settings(), authority: null }));
    renderPage();
    await screen.findByText('EPAS1');
    expect(screen.queryByTestId('dictionary-add')).toBeNull();
    expect(screen.queryByTestId('dictionary-row-edit')).toBeNull();
    expect(screen.getByTestId('dictionary-export')).toBeEnabled();
    const row = screen.getByText('EPAS1').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(
      within(row).getByRole('button', { name: DICTIONARY_LABELS.moreActions('EPAS1') })
    );
    expect(screen.getByTestId('dictionary-row-delete')).toHaveAttribute('data-disabled');
    await userEvent.keyboard('{Escape}');
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.pageMenu }));
    expect(screen.getByTestId('dictionary-delete-all')).toHaveAttribute('data-disabled');
    await userEvent.keyboard('{Escape}');
    // A suggestion's answers are on the row, not behind its menu: no Keep without write
    // authority, and Dismiss shown but refused before the click.
    const suggested = screen.getByText('Maybe').closest('[data-dictionary-row]') as HTMLElement;
    expect(within(suggested).queryByTestId('dictionary-row-accept')).toBeNull();
    expect(within(suggested).getByTestId('dictionary-row-dismiss')).toBeDisabled();
    await userEvent.click(within(suggested).getByTestId('dictionary-row-dismiss'));
    // The settings it used to close here moved to Settings, which closes them the same way
    // (`DictionarySettingsRows.test.tsx`).
    expect(api.addDictionaryEntry).not.toHaveBeenCalled();
    expect(api.editDictionaryEntry).not.toHaveBeenCalled();
    expect(api.deleteDictionaryEntry).not.toHaveBeenCalled();
    expect(api.dismissDictionarySuggestions).not.toHaveBeenCalled();
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('permits a browser public add while private add is denied by the host', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const reason = 'Only the host can add private terms.';
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: {
            ...settings().authority,
            add_private: { allowed: false, reason },
          },
        })
      )
    );
    api.addDictionaryEntry.mockResolvedValue(
      answer(201, { entry: entry({ written: 'EPAS1' }), created: true })
    );
    renderPage();
    await screen.findByText(DICTIONARY_MESSAGES['MSG-EMPTY']);
    await userEvent.click(screen.getAllByTestId('dictionary-add')[0]);
    await userEvent.click(screen.getByTestId('dictionary-add-term'));
    expect(screen.getByTestId('dictionary-term-private')).toBeDisabled();
    expect(screen.getByTestId('dictionary-term-private-help')).toHaveTextContent(reason);
    await userEvent.type(screen.getByTestId('dictionary-term-written'), 'EPAS1');
    await userEvent.click(screen.getByTestId('dictionary-term-save'));
    await waitFor(() => expect(api.addDictionaryEntry).toHaveBeenCalledTimes(1));
    expect(api.addDictionaryEntry.mock.calls[0][0].body).toMatchObject({ private: false });
  });

  it('uses daemon write authority on a serve browser', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    rows = [entry({ written: 'EPAS1' })];
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: {
            ...settings().authority,
            write: { allowed: false, reason: 'This host does not allow dictionary edits.' },
          },
        })
      )
    );
    renderPage();
    await screen.findByText('EPAS1');
    expect(screen.queryByTestId('dictionary-add')).toBeNull();
    expect(screen.queryByTestId('dictionary-row-edit')).toBeNull();
    expect(screen.getByTestId('dictionary-export')).toBeEnabled();
  });

  it('shows the daemon reason for a denied browser tier change before a click', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    rows = [entry({ written: 'EPAS1', tier: 'private' })];
    const reason = 'Only the host can change this term’s visibility.';
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: {
            ...settings().authority,
            tier_change: { allowed: false, reason },
          },
        })
      )
    );
    renderPage();
    const row = (await screen.findByText('EPAS1')).closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(
      within(row).getByRole('button', { name: DICTIONARY_LABELS.moreActions('EPAS1') })
    );
    expect(screen.getByTestId('dictionary-make-public')).toHaveAttribute('data-disabled');
    expect(screen.getByText(reason)).toBeVisible();
    expect(api.changeDictionaryEntryTier).not.toHaveBeenCalled();
  });

  it('shows MSG-EMPTY with the Add menu when there are no terms', async () => {
    renderPage();
    expect(await screen.findByText(DICTIONARY_MESSAGES['MSG-EMPTY'])).toBeInTheDocument();
    // The examples are UCSF and Biorouter (owner, 2026-11-19), as the CLI's help gives them.
    expect(DICTIONARY_MESSAGES['MSG-EMPTY']).toBe(
      'No terms yet. Add the words dictation should spell your way, such as UCSF or Biorouter.'
    );
    expect(screen.getAllByTestId('dictionary-add').length).toBeGreaterThan(0);
  });

  it('says so when a filter matches nothing', async () => {
    rows = [entry({ written: 'EPAS1' })];
    renderPage();
    await screen.findByText('EPAS1');
    await userEvent.type(screen.getByRole('searchbox', { name: DICTIONARY_LABELS.filter }), 'zzz');
    expect(
      await screen.findByText(DICTIONARY_MESSAGES['MSG-DICTIONARY-NO-MATCH'])
    ).toBeInTheDocument();
    // Titled for what happened, never the name of the list that is empty.
    expect(screen.getByText(DICTIONARY_LABELS.noMatches)).toBeInTheDocument();
    expect(screen.queryByText(DICTIONARY_LABELS.groups.matches)).not.toBeInTheDocument();
  });

  it('is read only under managed policy, with export and delete still offered', async () => {
    resetDictationStoreForTests(
      fakeStatusApi(() =>
        speechStatus({
          policy: {
            ...speechStatus().policy,
            managed: true,
            dictionary: { allowed: false, reason: null },
          },
        })
      ).api
    );
    await refreshDictationStatus();
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: {
            ...settings().authority,
            write: { allowed: false, reason: DICTATION_MESSAGES['MSG-POLICY'] },
            delete: { allowed: true, reason: null },
          },
        })
      )
    );
    rows = [entry({ written: 'EPAS1' }), entry({ written: 'Maybe', status: 'suggested' })];
    renderPage();
    await screen.findByText('EPAS1');
    expect(screen.getByTestId('dictionary-notice')).toHaveTextContent(
      DICTATION_MESSAGES['MSG-POLICY']
    );
    expect(screen.queryByTestId('dictionary-add')).toBeNull();
    expect(screen.queryByTestId('dictionary-row-edit')).toBeNull();
    expect(screen.queryByTestId('dictionary-row-accept')).toBeNull();
    expect(screen.queryByTestId('dictionary-review-all')).toBeNull();
    expect(screen.getByTestId('dictionary-export')).toBeEnabled();
    const row = screen.getByText('EPAS1').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(
      within(row).getByRole('button', { name: DICTIONARY_LABELS.moreActions('EPAS1') })
    );
    expect(screen.getByTestId('dictionary-row-delete')).not.toHaveAttribute('data-disabled');
    await userEvent.keyboard('{Escape}');
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.pageMenu }));
    expect(screen.getByTestId('dictionary-delete-all')).not.toHaveAttribute('data-disabled');
  });

  it('opens with no row ticked, so neither button acts before the person chooses', async () => {
    rows = [
      entry({ written: 'Biorouter', status: 'suggested' }),
      entry({ written: 'FOXP3', status: 'suggested' }),
    ];
    renderPage();
    await userEvent.click(await screen.findByTestId('dictionary-review-all'));
    const dialog = await screen.findByTestId('dictionary-review-dialog');
    for (const box of within(dialog).getAllByTestId('dictionary-review-row')) {
      expect(box).not.toBeChecked();
    }
    expect(within(dialog).getByTestId('dictionary-review-all')).not.toBeChecked();
    expect(within(dialog).getByTestId('dictionary-review-dismiss')).toBeDisabled();
    expect(within(dialog).getByTestId('dictionary-review-accept')).toBeDisabled();
    await userEvent.click(within(dialog).getByTestId('dictionary-review-dismiss'));
    expect(api.dismissDictionarySuggestions).not.toHaveBeenCalled();
  });

  it('reviews every waiting row at once: accepts the suggestions and keeps the agent’s rows', async () => {
    const suggestion = entry({ written: 'Suggested one', status: 'suggested', source: 'auto' });
    const agentRow = entry({ written: 'Agent new', source: 'agent', reviewed: false });
    rows = [suggestion, agentRow, entry({ written: 'Mine' })];
    renderPage();
    await userEvent.click(await screen.findByTestId('dictionary-review-all'));
    const dialog = await screen.findByTestId('dictionary-review-dialog');
    expect(within(dialog).getAllByTestId('dictionary-review-row')).toHaveLength(2);
    // Select all, then accept: two presses for every row.
    await userEvent.click(within(dialog).getByTestId('dictionary-review-all'));
    await userEvent.click(within(dialog).getByTestId('dictionary-review-accept'));
    await waitFor(() =>
      expect(api.acceptDictionarySuggestions).toHaveBeenCalledWith(
        expect.objectContaining({ body: { ids: [suggestion.id] } })
      )
    );
    expect(api.keepDictionaryEntries).toHaveBeenCalledWith(
      expect.objectContaining({ body: { ids: [agentRow.id] } })
    );
  });

  it('dismisses the chosen suggestions and removes the chosen agent rows', async () => {
    const one = entry({ written: 'One', status: 'suggested' });
    const two = entry({ written: 'Two', status: 'suggested' });
    const agentRow = entry({ written: 'Agent new', source: 'agent', reviewed: false });
    rows = [one, two, agentRow];
    renderPage();
    await userEvent.click(await screen.findByTestId('dictionary-review-all'));
    const dialog = await screen.findByTestId('dictionary-review-dialog');
    // Tick "One" and the agent's row, not "Two": only the chosen rows go.
    const [first, , third] = within(dialog).getAllByTestId('dictionary-review-row');
    await userEvent.click(first);
    await userEvent.click(third);
    await userEvent.click(within(dialog).getByTestId('dictionary-review-dismiss'));
    await waitFor(() => expect(api.dismissDictionarySuggestions).toHaveBeenCalled());
    const dismissed = api.dismissDictionarySuggestions.mock.calls[0][0].body.ids;
    expect(dismissed).toHaveLength(1);
    expect(api.deleteDictionaryEntries).toHaveBeenCalledWith(
      expect.objectContaining({ body: { ids: [agentRow.id] } })
    );
  });

  it('draws the first lines of a dictionary at its cap and holds the rest in placeholders', async () => {
    rows = Array.from({ length: 5_000 }, (_, index) =>
      entry({ written: `GENE${String(index).padStart(4, '0')}`, source: 'import' })
    );
    renderPage();
    await screen.findByText('GENE0000');
    const drawnRows = document.querySelectorAll('[data-dictionary-row]').length;
    expect(drawnRows).toBeGreaterThan(0);
    expect(drawnRows).toBeLessThan(60);
    const heldRows = [...document.querySelectorAll<HTMLElement>('[data-dictionary-gap]')].reduce(
      (sum, gap) => sum + Number(gap.dataset.dictionaryGap),
      0
    );
    expect(drawnRows + heldRows).toBe(5_000);
    expect(screen.getByRole('heading', { name: /Imported 5000/ })).toBeInTheDocument();
  });

  it('asks the warning the daemon picks before Make public…', async () => {
    rows = [
      entry({ written: 'From a chat', tier: 'private', came_from_a_chat: true }),
      entry({ written: 'Made private', tier: 'private', came_from_a_chat: false }),
    ];
    renderPage();
    const open = async (term: string) => {
      const row = (await screen.findByText(term)).closest('[data-dictionary-row]') as HTMLElement;
      await userEvent.click(
        within(row).getByRole('button', { name: DICTIONARY_LABELS.moreActions(term) })
      );
      await userEvent.click(
        await screen.findByRole('menuitem', { name: DICTIONARY_LABELS.makePublic })
      );
    };
    await open('From a chat');
    expect(await screen.findByText(DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN'])).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.cancel }));
    await open('Made private');
    expect(
      await screen.findByText(DICTIONARY_MESSAGES['MSG-PUBLICIZE-WARN-NO-CHAT'])
    ).toBeInTheDocument();
  });
});

describe('no row moves under a hovering pointer', () => {
  it('holds the rows while the pointer is over the list, and catches up when it leaves', async () => {
    rows = [entry({ written: 'Bravo' }), entry({ written: 'Charlie' })];
    renderPage();
    await screen.findByText('Bravo');
    const list = screen.getByTestId('dictionary-groups');
    fireEvent.pointerEnter(list);
    expect(list).toHaveAttribute('data-holding', 'true');

    rows = [entry({ written: 'Alpha' }), ...rows];
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(api.listDictionaryEntries.mock.calls.length).toBeGreaterThan(1));
    expect(screen.queryByText('Alpha')).toBeNull();

    fireEvent.pointerLeave(list);
    expect(await screen.findByText('Alpha')).toBeInTheDocument();
  });

  it('counts in the band the rows it shows, which the hold keeps, not the rows waiting behind it', async () => {
    rows = [entry({ written: 'Bravo' }), entry({ written: 'Charlie' })];
    renderPage();
    await screen.findByText('Bravo');
    const count = () =>
      document.querySelector('.biorouter-page-header-adornment')?.textContent?.trim();
    expect(count()).toBe('2');
    const list = screen.getByTestId('dictionary-groups');
    fireEvent.pointerEnter(list);

    rows = [
      ...rows,
      entry({ written: 'Biorouter', status: 'suggested' }),
      entry({ written: 'FOXP3', status: 'suggested' }),
    ];
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(api.listDictionaryEntries.mock.calls.length).toBeGreaterThan(1));
    // The Suggested group waits for the pointer to leave, and so does the count.
    expect(sections()).toEqual(['user']);
    expect(count()).toBe('2');

    fireEvent.pointerLeave(list);
    await waitFor(() => expect(sections()).toEqual(['suggested', 'user']));
    expect(count()).toBe('4');
  });

  it('keeps a kept suggestion in place, saying so, until the pointer leaves', async () => {
    const suggestion = entry({ written: 'EPAS1', status: 'suggested' });
    rows = [suggestion, entry({ written: 'Next one', status: 'suggested' })];
    renderPage();
    await screen.findByText('EPAS1');
    const list = screen.getByTestId('dictionary-groups');
    fireEvent.pointerEnter(list);
    api.listDictionaryEntries.mockImplementation(async () =>
      listing([{ ...suggestion, status: 'active', reviewed: true }, rows[1]])
    );
    const row = screen.getByText('EPAS1').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(within(row).getByRole('button', { name: 'Keep EPAS1' }));
    await waitFor(() => expect(row).toHaveAttribute('data-settled', 'accepted'));
    expect(within(row).getByRole('status')).toHaveTextContent(DICTIONARY_LABELS.settled.accepted);
    expect(row.closest('[data-dictionary-section]')).toHaveAttribute(
      'data-dictionary-section',
      'suggested'
    );

    fireEvent.pointerLeave(list);
    await waitFor(() => {
      const moved = screen.getByText('EPAS1').closest('[data-dictionary-section]');
      expect(moved).toHaveAttribute('data-dictionary-section', 'user');
    });
  });
});

describe('a waiting row’s answers', () => {
  it('shows Keep and Dismiss on a suggestion at rest, outside the hover cluster, and sends each', async () => {
    const epas = entry({ written: 'EPAS1', status: 'suggested', source: 'auto' });
    const hif = entry({ written: 'HIF1A', status: 'suggested', source: 'auto' });
    rows = [epas, hif];
    renderPage();
    const row = (await screen.findByText('EPAS1')).closest('[data-dictionary-row]') as HTMLElement;
    const keep = within(row).getByRole('button', { name: 'Keep EPAS1' });
    const dismiss = within(row).getByRole('button', { name: 'Dismiss EPAS1' });
    expect(keep).toHaveTextContent(DICTIONARY_LABELS.keep);
    expect(dismiss).toHaveTextContent(DICTIONARY_LABELS.dismiss);
    // Always visible: not in the cluster that shows only on hover or focus.
    expect(keep.closest('.br-row-actions')).toBeNull();
    expect(dismiss.closest('.br-row-actions')).toBeNull();
    expect(within(row).queryByText('Accept')).toBeNull();
    await userEvent.click(keep);
    await waitFor(() =>
      expect(api.acceptDictionarySuggestions).toHaveBeenCalledWith(
        expect.objectContaining({ body: { ids: [epas.id] }, headers: { 'X-User-Action': 'proof' } })
      )
    );
    const other = screen.getByText('HIF1A').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(within(other).getByRole('button', { name: 'Dismiss HIF1A' }));
    await waitFor(() =>
      expect(api.dismissDictionarySuggestions).toHaveBeenCalledWith(
        expect.objectContaining({ body: { ids: [hif.id] }, headers: { 'X-User-Action': 'proof' } })
      )
    );
  });

  it('keeps Edit…, Copy term and the chat in the ⋯ menu, and no answer there', async () => {
    rows = [entry({ written: 'EPAS1', status: 'suggested', source: 'auto', suggested_in: 's1' })];
    renderPage();
    const row = (await screen.findByText('EPAS1')).closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(
      within(row).getByRole('button', { name: DICTIONARY_LABELS.moreActions('EPAS1') })
    );
    expect(screen.getByRole('menuitem', { name: DICTIONARY_LABELS.editMenu })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: DICTIONARY_LABELS.copyTerm })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: DICTIONARY_LABELS.openChat })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: DICTIONARY_LABELS.dismiss })).toBeNull();
  });

  it('shows the agent’s unreviewed row the same way: Keep and Remove at rest', async () => {
    const agentRow = entry({ written: 'Agent new', source: 'agent', reviewed: false });
    rows = [agentRow];
    renderPage();
    const row = (await screen.findByText('Agent new')).closest(
      '[data-dictionary-row]'
    ) as HTMLElement;
    const keep = within(row).getByTestId('dictionary-row-keep');
    expect(keep).toHaveAccessibleName('Keep Agent new');
    expect(keep.closest('.br-row-actions')).toBeNull();
    expect(within(row).getByTestId('dictionary-row-remove')).toHaveAccessibleName(
      'Remove Agent new'
    );
    await userEvent.click(keep);
    await waitFor(() =>
      expect(api.keepDictionaryEntries).toHaveBeenCalledWith(
        expect.objectContaining({ body: { ids: [agentRow.id] } })
      )
    );
  });

  it('says Kept in place for a suggestion kept under the pointer', async () => {
    const suggestion = entry({ written: 'EPAS1', status: 'suggested' });
    rows = [suggestion];
    renderPage();
    await screen.findByText('EPAS1');
    fireEvent.pointerEnter(screen.getByTestId('dictionary-groups'));
    const row = screen.getByText('EPAS1').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(within(row).getByRole('button', { name: 'Keep EPAS1' }));
    await waitFor(() => expect(within(row).getByRole('status')).toHaveTextContent('Kept'));
    expect(within(row).queryByRole('button', { name: 'Keep EPAS1' })).toBeNull();
  });

  it('asks Keep selected and Dismiss selected in the review dialog, under the keep sentence', async () => {
    rows = [entry({ written: 'EPAS1', status: 'suggested' })];
    renderPage();
    await userEvent.click(await screen.findByTestId('dictionary-review-all'));
    const dialog = await screen.findByTestId('dictionary-review-dialog');
    expect(within(dialog).getByTestId('dictionary-review-accept')).toHaveTextContent(
      'Keep selected'
    );
    expect(within(dialog).getByTestId('dictionary-review-dismiss')).toHaveTextContent(
      'Dismiss selected'
    );
    expect(
      within(dialog).getByText(
        'A kept word starts shaping dictation. A dismissed one is not suggested again.'
      )
    ).toBeInTheDocument();
  });
});

describe('the offer to turn suggestions off', () => {
  const offer = () =>
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          learn_from_chats: true,
          suggestions: {
            recent_kept: 3,
            recent_decisions: 20,
            required_chats: 3,
            offer_turning_off: true,
          },
        })
      )
    );

  it('is not shown while the person keeps most suggestions', async () => {
    api.getDictionarySettings.mockResolvedValue(answer(200, settings({ learn_from_chats: true })));
    renderPage();
    await screen.findByText(DICTIONARY_MESSAGES['MSG-EMPTY']);
    await waitFor(() => expect(api.getDictionarySettings).toHaveBeenCalled());
    expect(screen.queryByTestId('dictionary-back-off')).toBeNull();
  });

  it('says so in one line with its actions, and how many chats a word needs in its InfoTip', async () => {
    offer();
    renderPage();
    const note = await screen.findByTestId('dictionary-back-off');
    expect(within(note).getByTestId('dictionary-back-off-line')).toHaveTextContent(
      /^You dismissed most suggestions, so Biorouter now suggests fewer\.$/
    );
    const help = within(note).getByTestId('dictionary-back-off-help');
    expect(help).toHaveAccessibleName('About fewer suggestions');
    expect(note).toHaveTextContent('A word now has to show up in 3 chats before it is suggested.');
    expect(within(note).getByRole('button', { name: 'Turn off suggestions' })).toBeInTheDocument();
    expect(within(note).getByRole('button', { name: 'Keep suggesting' })).toBeInTheDocument();
    expect(note).toHaveClass('bg-background-fill');
    expect(note).not.toHaveClass('bg-wash-danger');
    expect(note).not.toHaveClass('bg-wash-warning');
  });

  it('turns suggestions off', async () => {
    offer();
    renderPage();
    const note = await screen.findByTestId('dictionary-back-off');
    await userEvent.click(within(note).getByRole('button', { name: 'Turn off suggestions' }));
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { learn_from_chats: false },
          headers: { 'X-User-Action': 'proof' },
        })
      )
    );
  });

  it('keeps suggesting', async () => {
    offer();
    renderPage();
    const note = await screen.findByTestId('dictionary-back-off');
    const settingsReads = api.getDictionarySettings.mock.calls.length;
    await userEvent.click(within(note).getByRole('button', { name: 'Keep suggesting' }));
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { keep_suggesting: true },
          headers: { 'X-User-Action': 'proof' },
        })
      )
    );
    // The settings are read again, so the note follows the daemon's answer.
    await waitFor(() =>
      expect(api.getDictionarySettings.mock.calls.length).toBeGreaterThan(settingsReads)
    );
  });
});

/**
 * The page holds the words alone (owner, 2026-11-19): how they are found is in Settings
 * (`settings/models/DictionarySettingsRows.test.tsx`), and the page's `⋯` menu opens it.
 */
describe('the page holds the words alone', () => {
  function SettingsProbe() {
    const location = useLocation();
    return <div data-testid="settings-probe">{JSON.stringify(location.state)}</div>;
  }

  function renderWithSettings() {
    return render(
      <MemoryRouter initialEntries={['/dictionary']}>
        <Routes>
          <Route path="/dictionary" element={<DictionaryView />} />
          <Route path="/settings" element={<SettingsProbe />} />
        </Routes>
      </MemoryRouter>
    );
  }

  it('shows no settings and no scan, even with suggestions on', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(200, settings({ learn_from_chats: true, suggest_from_chats: true }))
    );
    rows = [entry({ written: 'EPAS1' })];
    renderPage();
    await screen.findByText('EPAS1');
    await waitFor(() => expect(api.getDictionarySettings).toHaveBeenCalled());
    for (const name of [
      DICTIONARY_LABELS.agentSettings.title,
      'Suggest dictionary words from my chats',
      DICTIONARY_LABELS.agentSettings.learnFromChats,
      DICTIONARY_LABELS.scan.find,
    ]) {
      expect(screen.queryByText(name), name).toBeNull();
    }
    expect(screen.queryByRole('checkbox')).toBeNull();
    expect(screen.queryByTestId('dictionary-agent-settings')).toBeNull();
    expect(screen.queryByTestId('dictionary-scan-start')).toBeNull();
    expect(api.getDictionarySuggestionScan).not.toHaveBeenCalled();
  });

  it('opens the dictionary rows in Settings from the band, its one link there', async () => {
    renderWithSettings();
    await screen.findByText(DICTIONARY_MESSAGES['MSG-EMPTY']);
    const link = screen.getByRole('button', { name: DICTIONARY_LABELS.settingsLink });
    expect(link).toHaveAttribute('data-testid', 'dictionary-settings-link');
    // One link: the ⋯ menu no longer repeats it.
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.pageMenu }));
    expect(await screen.findByTestId('dictionary-delete-all')).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: DICTIONARY_LABELS.settingsLink })).toBeNull();
    await userEvent.keyboard('{Escape}');
    await userEvent.click(link);
    expect(await screen.findByTestId('settings-probe')).toHaveTextContent(
      JSON.stringify({ section: 'dictionary' })
    );
  });

  it('offers no settings link where managed policy turns the dictionary off', async () => {
    resetDictationStoreForTests(
      fakeStatusApi(() =>
        speechStatus({
          policy: {
            ...speechStatus().policy,
            managed: true,
            dictionary: { allowed: false, reason: null },
          },
        })
      ).api
    );
    await refreshDictationStatus();
    rows = [entry({ written: 'EPAS1' })];
    renderPage();
    await screen.findByText('EPAS1');
    await userEvent.click(screen.getByRole('button', { name: DICTIONARY_LABELS.pageMenu }));
    expect(await screen.findByTestId('dictionary-delete-all')).toBeInTheDocument();
    expect(screen.queryByTestId('dictionary-settings-link')).toBeNull();
  });
});

describe('the band’s tabs (DD-96)', () => {
  it('opens on Terms with its own band, and reads nothing of the history there', async () => {
    rows = [entry({ written: 'EPAS1' })];
    renderPage();
    await screen.findByText('EPAS1');
    expect(screen.getByRole('tab', { name: DICTIONARY_LABELS.tabs.terms })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByRole('tab', { name: DICTIONARY_LABELS.tabs.history })).toHaveAttribute(
      'aria-selected',
      'false'
    );
    expect(screen.getByTestId('dictionary-add')).toHaveTextContent(DICTIONARY_LABELS.add);
    expect(screen.getByRole('searchbox', { name: /Filter terms/ })).toBeInTheDocument();
    expect(api.listDictationHistory).not.toHaveBeenCalled();
    expect(api.getDictationHistorySettings).not.toHaveBeenCalled();
  });

  it('keeps the Terms filter and the Add menu out of the History band', async () => {
    api.listDictationHistory.mockResolvedValue({
      data: JSON.stringify({ entries: [], next_cursor: null, total: 0 }),
      response: new Response('{}', { status: 200 }),
    });
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, {
        keep_history: true,
        public_by_default: false,
        authority: {
          list_private: { allowed: true, reason: null },
          write: { allowed: true, reason: null },
          tier_change: { allowed: true, reason: null },
          raise_settings: { allowed: true, reason: null },
        },
      })
    );
    renderPage();
    await userEvent.click(screen.getByRole('tab', { name: DICTIONARY_LABELS.tabs.history }));
    expect(await screen.findByTestId('history-tab')).toBeInTheDocument();
    expect(screen.queryByTestId('dictionary-add')).toBeNull();
    expect(screen.queryByRole('searchbox', { name: /Filter terms/ })).toBeNull();
    expect(screen.getByRole('searchbox', { name: /Search dictations/ })).toBeInTheDocument();
  });
});
