import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, within } from '@testing-library/react';
import MemorySection from './MemorySection';
import { memoryCopy } from './copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';

const mockInventory = vi.fn();
const mockDeleteEntry = vi.fn();
const mockDeleteCategory = vi.fn();

vi.mock('../../../api', () => ({
  memoryInventory: (...args: unknown[]) => mockInventory(...args),
  memoryDeleteEntry: (...args: unknown[]) => mockDeleteEntry(...args),
  memoryDeleteCategory: (...args: unknown[]) => mockDeleteCategory(...args),
}));

const mockToastSuccess = vi.fn();
const mockToastError = vi.fn();
vi.mock('../../../toasts', () => ({
  toastSuccess: (...args: unknown[]) => mockToastSuccess(...args),
  toastError: (...args: unknown[]) => mockToastError(...args),
}));

vi.mock('../../../utils/workingDir', () => ({
  getInitialWorkingDir: () => '/Users/someone/work/proj',
}));

const entry = (index: number, content: string, tags: string[] = []) => ({
  index,
  tags,
  content,
  // The row's identity, over its tags as well as its body. The delete carries
  // it back so a list that went stale cannot take out the wrong memory.
  digest: `digest-${index}-${content.slice(0, 8)}`,
});

const inventoryResponse = (overrides: Record<string, unknown> = {}) => ({
  data: {
    global: {
      scope: 'global',
      path: '/Users/someone/.config/biorouter/memory',
      exists: true,
      categories: [
        {
          name: 'clinical',
          entries: [
            entry(0, 'the cohort has 812 patients', ['phi', 'cohort']),
            entry(1, 'exclusions are documented in the protocol'),
          ],
          revision: 'clinical-rev-1',
          size_bytes: 412,
          modified: 1_780_000_000,
        },
      ],
    },
    local: {
      scope: 'local',
      path: '/Users/someone/work/proj/.biorouter/memory',
      exists: true,
      categories: [
        {
          name: 'development',
          entries: [entry(0, 'we format with black')],
          revision: 'development-rev-1',
          size_bytes: 24,
          modified: 1_780_000_000,
        },
      ],
    },
    ...overrides,
  },
});

beforeEach(() => {
  vi.clearAllMocks();
  mockInventory.mockResolvedValue(inventoryResponse());
  mockDeleteEntry.mockResolvedValue({ data: { remaining: 1, category_removed: false } });
  mockDeleteCategory.mockResolvedValue({ data: { removed_entries: 2 } });
});

describe('MemorySection', () => {
  it('shows what has been remembered in both stores, with its provenance', async () => {
    render(<MemorySection />);

    // The machine-wide store, which is the one the #63 consent gate asks about.
    expect(await screen.findByText('clinical')).toBeTruthy();
    expect(screen.getByText('/Users/someone/.config/biorouter/memory')).toBeTruthy();

    // Contents are visible, not just category names.
    fireEvent.click(screen.getByRole('button', { name: /show the 2 memories in clinical/i }));
    expect(await screen.findByText('the cohort has 812 patients')).toBeTruthy();
    expect(screen.getByText('exclusions are documented in the protocol')).toBeTruthy();
    // Tags are the only per-entry provenance the store carries.
    expect(screen.getByText('phi')).toBeTruthy();
    expect(screen.getByText('cohort')).toBeTruthy();

    // The project store is listed separately, and named.
    expect(screen.getByText('development')).toBeTruthy();
    expect(screen.getByText('/Users/someone/work/proj/.biorouter/memory')).toBeTruthy();
  });

  it('passes the window project so the local store is this project’s', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');
    expect(mockInventory).toHaveBeenCalledWith(
      expect.objectContaining({ query: { working_dir: '/Users/someone/work/proj' } })
    );
  });

  it('says the category is machine-wide, how many memories go, and that it cannot be undone', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');

    fireEvent.click(screen.getByRole('button', { name: /delete the global category clinical/i }));

    const dialog = await screen.findByRole('dialog');
    const text = dialog.textContent ?? '';
    expect(text).toContain('clinical');
    expect(text).toContain('2 memories');
    expect(text.toLowerCase()).toContain('every chat');
    expect(text.toLowerCase()).toContain('cannot be undone');
    // Nothing is deleted by opening the confirmation.
    expect(mockDeleteCategory).not.toHaveBeenCalled();
  });

  it('deletes a category only after the confirmation is accepted', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');

    fireEvent.click(screen.getByRole('button', { name: /delete the global category clinical/i }));
    await screen.findByRole('dialog');
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() =>
      expect(mockDeleteCategory).toHaveBeenCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            scope: 'global',
            category: 'clinical',
            // The compare-and-set token: the daemon refuses if the category
            // moved since it was listed.
            revision: 'clinical-rev-1',
          }),
        })
      )
    );
  });

  it('quotes the memory it is about to delete, and sends the guard back with it', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');
    fireEvent.click(screen.getByRole('button', { name: /show the 2 memories in clinical/i }));

    fireEvent.click(
      (await screen.findAllByRole('button', { name: /delete this memory from clinical/i }))[0]
    );

    const dialog = await screen.findByRole('dialog');
    expect(dialog.textContent).toContain('the cohort has 812 patients');
    expect((dialog.textContent ?? '').toLowerCase()).toContain('cannot be undone');

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() =>
      expect(mockDeleteEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          body: {
            scope: 'global',
            category: 'clinical',
            index: 0,
            digest: 'digest-0-the coho',
            revision: 'clinical-rev-1',
            working_dir: '/Users/someone/work/proj',
          },
        })
      )
    );
  });

  it('warns that deleting the last memory takes the category with it', async () => {
    render(<MemorySection />);
    await screen.findByText('development');
    fireEvent.click(screen.getByRole('button', { name: /show the 1 memory in development/i }));

    fireEvent.click(
      (await screen.findAllByRole('button', { name: /delete this memory from development/i }))[0]
    );

    const dialog = await screen.findByRole('dialog');
    expect((dialog.textContent ?? '').toLowerCase()).toContain('last memory');
  });

  it('tells the user there is no project rather than showing an empty local store', async () => {
    mockInventory.mockResolvedValue(inventoryResponse({ local: null }));
    render(<MemorySection />);
    await screen.findByText('clinical');

    expect(screen.getByText(/no project/i)).toBeTruthy();
  });

  it('says plainly when nothing has been remembered globally', async () => {
    mockInventory.mockResolvedValue(
      inventoryResponse({
        global: {
          scope: 'global',
          path: '/Users/someone/.config/biorouter/memory',
          exists: false,
          categories: [],
        },
      })
    );
    render(<MemorySection />);

    expect(await screen.findByText(/nothing has been remembered/i)).toBeTruthy();
  });

  it('surfaces a refused delete instead of pretending it worked', async () => {
    mockDeleteCategory.mockRejectedValue(new Error('the category changed since it was listed'));
    render(<MemorySection />);
    await screen.findByText('clinical');

    fireEvent.click(screen.getByRole('button', { name: /delete the global category clinical/i }));
    await screen.findByRole('dialog');
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(mockToastError).toHaveBeenCalled());
    // The row is still there: a failed delete must not disappear from the list.
    expect(screen.getByText('clinical')).toBeTruthy();
  });

  it('a failed load says so, and does not claim there is no project', async () => {
    mockInventory.mockRejectedValue(new Error('the daemon is not answering'));
    render(<MemorySection />);

    expect(await screen.findByText(/the daemon is not answering/i)).toBeTruthy();
    // "No project open" is a real, different state. Reporting it when the
    // request simply failed tells the user their memories are not there,
    // which for a store they are being asked to consent to reads of is the
    // one wrong answer.
    expect(screen.queryByText(/no project/i)).toBeNull();
    expect(screen.queryByText(/nothing has been remembered/i)).toBeNull();
  });

  it('a long category name and a long memory body cannot break the layout', async () => {
    const long = 'a'.repeat(400);
    mockInventory.mockResolvedValue(
      inventoryResponse({
        global: {
          scope: 'global',
          path: '/Users/someone/.config/biorouter/memory',
          exists: true,
          categories: [
            {
              name: long,
              entries: [entry(0, 'x'.repeat(2000))],
              revision: 'long-rev-1',
              size_bytes: 2000,
              modified: 1_780_000_000,
            },
          ],
        },
      })
    );
    render(<MemorySection />);

    const name = await screen.findByText(long);
    // A flex/grid item defaults to min-width:auto, which is what lets long
    // unbroken text push a row wider than its container; min-w-0 is the fix and
    // break-words alone is not enough.
    expect(name.className).toContain('min-w-0');
    expect(name.className).toContain('[overflow-wrap:anywhere]');

    fireEvent.click(screen.getByRole('button', { name: `Show the 1 memory in ${long}` }));
    const body = await screen.findByText('x'.repeat(2000));
    expect(body.className).toContain('[overflow-wrap:anywhere]');
  });
});

// Spec 3.14: Memory is one `SettingSection` of one-line rows. The header carries its explanation
// as an InfoTip and Refresh as a glyph; each store is a heading with a right-aligned count and an
// InfoTip (who can read it, where it lives); an empty store is one muted row; an open category's
// entries are sibling rows; and nothing is red until the person points at it.
describe('MemorySection layout', () => {
  it('is a section named Memory, reachable by its deep-link id, with its help as a tip', async () => {
    const { container } = render(<MemorySection />);
    await screen.findByText('clinical');

    const section = screen.getByRole('region', { name: memoryCopy.section });
    expect(section.id).toBe(SETTINGS_SECTION_IDS.memory);
    expect(container.querySelector('[data-testid="memory-section"]')).toBe(section);
    expect(
      within(section).getByRole('button', { name: `About ${memoryCopy.section}` })
    ).toHaveAccessibleDescription(memoryCopy.help);
    // The header keeps no paragraph.
    expect(section.querySelectorAll('.biorouter-settings-section-header p')).toHaveLength(0);
  });

  it('refreshes from a glyph with a name, not a labelled button', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');
    expect(mockInventory).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('button', { name: memoryCopy.refresh }));
    await waitFor(() => expect(mockInventory).toHaveBeenCalledTimes(2));
    expect(screen.queryByText('Refresh')).toBeNull();
  });

  it('heads each store with its short name, a right-aligned count and a tip', async () => {
    const { container } = render(<MemorySection />);
    await screen.findByText('clinical');

    expect(screen.getByRole('heading', { name: memoryCopy.global.heading })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: memoryCopy.local.heading })).toBeInTheDocument();
    const counts = [...container.querySelectorAll('.br-memory-store-count')].map(
      (node) => node.textContent
    );
    expect(counts).toEqual(['2 memories', '1 memory']);

    // The audience sentence and the folder are the tip's description, so they are heard on
    // reaching the glyph and are not standing text.
    const globalTip = screen.getByRole('button', { name: 'About Global' });
    expect(globalTip).toHaveAccessibleDescription(
      expect.stringContaining('Shared by every chat on this computer')
    );
    expect(globalTip).toHaveAccessibleDescription(
      expect.stringContaining('/Users/someone/.config/biorouter/memory')
    );
    expect(screen.getByRole('button', { name: 'About This project' })).toHaveAccessibleDescription(
      expect.stringContaining('/Users/someone/work/proj/.biorouter/memory')
    );
    // The old sentence-long headings are gone.
    expect(screen.queryByText(/Global: shared by every chat/)).toBeNull();
    expect(screen.queryByText(/This project: only chats opened here/)).toBeNull();
  });

  it('shows a category as one line: its name, and only its count at the trailing edge', async () => {
    render(<MemorySection />);
    const name = await screen.findByText('clinical');
    const row = name.closest('.biorouter-settings-row') as HTMLElement;

    expect(row).toHaveTextContent('clinical');
    expect(row.querySelector('.br-row-actions-meta')).toHaveTextContent('2 memories');
    // Size and date are the name's tooltip, not text on the row.
    expect(row.textContent).not.toMatch(/\d+(\.\d)? (B|KB)/);
    expect(row.textContent).not.toMatch(/Updated/);
  });

  it('draws an open category’s entries as sibling rows, not inside the category row', async () => {
    render(<MemorySection />);
    const name = await screen.findByText('clinical');
    const categoryRow = name.closest('.biorouter-settings-row') as HTMLElement;

    fireEvent.click(screen.getByRole('button', { name: /show the 2 memories in clinical/i }));
    const body = await screen.findByText('the cohort has 812 patients');
    const entryRow = body.closest('.biorouter-settings-row') as HTMLElement;

    expect(entryRow).not.toBe(categoryRow);
    expect(categoryRow.contains(entryRow)).toBe(false);
    expect(categoryRow.nextElementSibling).toBe(entryRow);
    expect(entryRow.className).toContain('br-memory-entry');
    // Entry text is the 13px role, one step under the 14px category name.
    expect(body.className).toContain('text-secondary');
    expect(body.className).not.toContain('text-supporting');
  });

  it('keeps delete out of sight and out of red until the person means it', async () => {
    render(<MemorySection />);
    await screen.findByText('clinical');
    fireEvent.click(screen.getByRole('button', { name: /show the 2 memories in clinical/i }));

    const buttons = [
      screen.getByRole('button', { name: /delete the global category clinical/i }),
      ...screen.getAllByRole('button', { name: /delete this memory from clinical/i }),
    ];
    expect(buttons).toHaveLength(3);
    for (const button of buttons) {
      // Inside the cluster that stays at opacity 0 until the row is hovered or focused, and
      // that is always shown on a device without hover.
      expect(button.closest('.br-row-actions')).not.toBeNull();
      // Muted at rest (`.br-memory-delete`), never the standing red of the old column.
      expect(button.className).toContain('br-memory-delete');
      expect(button.className).not.toContain('text-text-danger');
      expect(button).toHaveAttribute('data-row-action');
    }
  });

  it('says an empty store in one muted row and keeps the rest in a tip', async () => {
    mockInventory.mockResolvedValue(
      inventoryResponse({
        global: {
          scope: 'global',
          path: '/Users/someone/.config/biorouter/memory',
          exists: false,
          categories: [],
        },
      })
    );
    render(<MemorySection />);

    const line = await screen.findByText(memoryCopy.empty);
    expect(line.closest('.br-memory-empty')).not.toBeNull();
    expect(line.closest('.br-memory-empty')!.querySelector('button')).toHaveAccessibleDescription(
      memoryCopy.emptyHelp
    );
    // A Note (a bordered box) is not used for an empty list.
    expect(line.closest('[class*="bg-background-muted"]')).toBeNull();
  });

  it('says "No project open" in one muted row, with the explanation as its tip', async () => {
    mockInventory.mockResolvedValue(inventoryResponse({ local: null }));
    render(<MemorySection />);
    await screen.findByText('clinical');

    const line = screen.getByText(memoryCopy.noProject);
    expect(line.closest('.br-memory-empty')).not.toBeNull();
    expect(
      screen.getByRole('button', { name: `About ${memoryCopy.noProject}` })
    ).toHaveAccessibleDescription(memoryCopy.noProjectHelp);
    // The store still has its heading, so the page does not lose a section.
    expect(screen.getByRole('heading', { name: memoryCopy.local.heading })).toBeInTheDocument();
  });
});
