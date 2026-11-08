import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictionaryEntryView } from '../../api';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { DICTATION_MESSAGES } from '../dictation/copy';
import { refreshDictationStatus, resetDictationStoreForTests } from '../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from './copy';
import { answer, entry, listing, settings } from './dictionaryTestKit';
import { groupEntries, lineTwo, privateLine, reviewRows, usesMeta } from './rows';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
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

let rows: DictionaryEntryView[] = [];

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  for (const fn of Object.values(api)) fn.mockReset();
  rows = [];
  api.listDictionaryEntries.mockImplementation(async () => listing(rows));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.acceptDictionarySuggestions.mockResolvedValue(answer(200, { accepted: 1 }));
  api.dismissDictionarySuggestions.mockResolvedValue(answer(200, { dismissed: 1 }));
  api.keepDictionaryEntries.mockResolvedValue(answer(200, { kept: 1 }));
  api.deleteDictionaryEntries.mockResolvedValue(answer(200, { deleted: 1 }));
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
    const suggested = screen.getByText('Maybe').closest('[data-dictionary-row]') as HTMLElement;
    await userEvent.click(
      within(suggested).getByRole('button', { name: DICTIONARY_LABELS.moreActions('Maybe') })
    );
    expect(screen.getByTestId('dictionary-row-dismiss')).toHaveAttribute('data-disabled');
    expect(api.addDictionaryEntry).not.toHaveBeenCalled();
    expect(api.editDictionaryEntry).not.toHaveBeenCalled();
    expect(api.deleteDictionaryEntry).not.toHaveBeenCalled();
    expect(api.dismissDictionarySuggestions).not.toHaveBeenCalled();
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

  it('keeps an accepted suggestion in place, saying so, until the pointer leaves', async () => {
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
    await userEvent.click(within(row).getByRole('button', { name: 'Accept EPAS1' }));
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
