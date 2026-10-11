import { createRef } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import MentionPopover, {
  type DisplayItem,
  type DisplayItemWithMatch,
  mentionRowDetail,
} from './MentionPopover';

/**
 * Files and folders in the mention menu (vision round 1, V2-10).
 *
 * Two things went wrong on one screen. Every row said the full absolute path under
 * its name, which made each row 46px and read as a wall of paths; and a file whose
 * extension the scanner did not know (a `.zip`) was drawn as a folder, because the
 * scanner asked `listFiles` whether an entry was a folder and `listFiles` answers an
 * empty list, not an error, for a file.
 */
const mocks = vi.hoisted(() => ({
  listFiles: vi.fn(),
  listSkillDirs: vi.fn(),
  // ONE array, as the real context's state is: a new one per render would make
  // `loadReferenceItems` new per render and restart the load on every render.
  extensionsList: [] as never[],
}));

vi.mock('../api', () => ({
  getActive: vi.fn(async () => ({ data: null })),
  listBases: vi.fn(async () => ({ data: [] })),
  getSlashCommands: vi.fn(async () => ({ data: { commands: [] } })),
  getSessionExtensions: vi.fn(async () => ({ data: { extensions: [] } })),
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ extensionsList: mocks.extensionsList }),
}));

vi.mock('./skills/useSkillCatalog', () => ({
  fetchSkillCatalog: vi.fn(async () => ({ generation: 0, roots: [], skills: [], bundles: [] })),
  pickerBundles: () => [],
  standaloneSkills: () => [],
}));

/** A small tree under `/w`, listed the way the main process lists it. */
const TREE: Record<string, { files: string[]; folders: string[] }> = {
  '/w': {
    files: ['ex-src', 'ex-archive.zip', 'ex-linked', 'ex-empty', 'readme.md'],
    folders: ['ex-src', 'ex-empty'],
  },
  '/w/ex-src': { files: ['ex-inner.zip', 'ex-notes.md'], folders: [] },
  // A symlink to a folder is not a folder of its parent's listing, but it reads as one.
  '/w/ex-linked': { files: ['a.txt'], folders: [] },
};

function item(overrides: Partial<DisplayItem>): DisplayItem {
  return { name: 'x', extra: '', itemType: 'File', relativePath: 'x', ...overrides };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listFiles.mockImplementation(async (dir: string) => TREE[dir]?.files ?? []);
  mocks.listSkillDirs.mockImplementation(async (dir: string) => TREE[dir]?.folders ?? []);
  Object.assign(window, {
    electron: {
      platform: 'darwin',
      getUserActionKey: vi.fn(async () => null),
      listFiles: mocks.listFiles,
      listSkillDirs: mocks.listSkillDirs,
    },
  });
});

describe('mentionRowDetail', () => {
  it('says nothing for a file or folder directly in the chat folder', () => {
    expect(
      mentionRowDetail(item({ itemType: 'File', name: 'a.md', relativePath: 'a.md' }))
    ).toBeUndefined();
    expect(
      mentionRowDetail(item({ itemType: 'Directory', name: 'src', relativePath: 'src' }))
    ).toBeUndefined();
  });

  it('names the folder a nested file or folder sits in, relative to the chat folder', () => {
    expect(
      mentionRowDetail(
        item({
          itemType: 'File',
          name: 'a.zip',
          relativePath: 'skills/a.zip',
          extra: '/w/skills/a.zip',
        })
      )
    ).toBe('skills');
    expect(
      mentionRowDetail(item({ itemType: 'Directory', name: 'c', relativePath: 'a/b/c' }))
    ).toBe('a/b');
  });

  it('never prints an absolute path', () => {
    const nested = item({
      itemType: 'File',
      name: 'a.zip',
      relativePath: 'skills/a.zip',
      extra: '/Users/someone/Desktop/skills/a.zip',
    });
    expect(mentionRowDetail(nested)).not.toContain('/Users');
  });

  it('leaves a command, skill or extension saying what it is', () => {
    expect(
      mentionRowDetail(item({ itemType: 'Builtin', name: 'compact', extra: 'Compact the chat' }))
    ).toBe('Compact the chat');
    expect(mentionRowDetail(item({ itemType: 'Skill', extra: 'Reviews a paper' }))).toBe(
      'Reviews a paper'
    );
    expect(mentionRowDetail(item({ itemType: 'Skill', extra: '' }))).toBeUndefined();
  });
});

describe('the scan of the chat folder', () => {
  async function scan() {
    const ref = createRef<{
      getDisplayFiles: () => DisplayItemWithMatch[];
      selectFile: (index: number) => void;
    }>();
    render(
      <MentionPopover
        ref={ref}
        isOpen
        isSlashCommand={false}
        query="ex"
        sessionId={null}
        workingDir="/w"
        position={{ x: 0, y: 400 }}
        selectedIndex={0}
        onSelectedIndexChange={() => {}}
        onSelect={vi.fn()}
        onClose={() => {}}
      />
    );
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(1));
    const byName = new Map((ref.current?.getDisplayFiles() ?? []).map((row) => [row.name, row]));
    return { byName };
  }

  it('calls a file with an unfamiliar extension a file, not a folder', async () => {
    const { byName } = await scan();
    expect(byName.get('ex-archive.zip')?.itemType).toBe('File');
    expect(byName.get('ex-inner.zip')?.itemType).toBe('File');
  });

  it('still calls a folder a folder, an empty one and a symlinked one included', async () => {
    const { byName } = await scan();
    expect(byName.get('ex-src')?.itemType).toBe('Directory');
    expect(byName.get('ex-empty')?.itemType).toBe('Directory');
    expect(byName.get('ex-linked')?.itemType).toBe('Directory');
  });

  it('takes the folders from the listing, one call per folder scanned', async () => {
    await scan();
    const asked = mocks.listSkillDirs.mock.calls.map(([dir]) => dir);
    expect(asked.filter((dir) => dir === '/w')).toHaveLength(1);
    expect(new Set(asked).size).toBe(asked.length);
  });

  it('keeps the full path as what a pick inserts', async () => {
    const { byName } = await scan();
    expect(byName.get('ex-inner.zip')?.extra).toBe('/w/ex-src/ex-inner.zip');
  });

  it('shows a name alone for a top level entry and the folder for a nested one', async () => {
    await scan();
    const rows = screen.getAllByRole('option');
    const rowOf = (name: string) => rows.find((row) => row.textContent?.startsWith(name));

    expect(rowOf('ex-archive.zip')?.textContent).toBe('ex-archive.zip');

    const nested = rowOf('ex-inner.zip');
    expect(nested?.textContent).toBe('ex-inner.zipex-src');
    expect(nested?.textContent).not.toContain('/w/');
  });

  it('puts the full path in the row tooltip', async () => {
    await scan();
    const nested = screen
      .getAllByRole('option')
      .find((row) => row.textContent?.startsWith('ex-inner.zip'));
    expect(nested).toHaveAttribute('title', '/w/ex-src/ex-inner.zip');
  });
});

/**
 * V2-R3-4: "Mention (@)" in the `+` menu, or an `@` typed alone, inserted the `@`
 * and drew nothing until two more characters were typed, while "Commands (/)"
 * listed at once. An `@` alone lists the chat folder now.
 */
describe('an "@" with little or nothing typed after it', () => {
  function renderMention(query: string, isSlashCommand = false) {
    const ref = createRef<{
      getDisplayFiles: () => DisplayItemWithMatch[];
      selectFile: (index: number) => void;
    }>();
    const onSelect = vi.fn();
    const menu = (next: string, slash: boolean) => (
      <MentionPopover
        ref={ref}
        isOpen
        isSlashCommand={slash}
        query={next}
        sessionId={null}
        workingDir="/w"
        position={{ x: 0, y: 400 }}
        selectedIndex={0}
        onSelectedIndexChange={() => {}}
        onSelect={onSelect}
        onClose={() => {}}
      />
    );
    const view = render(menu(query, isSlashCommand));
    const rerender = (next: string, slash = false) => view.rerender(menu(next, slash));
    return { ref, rerender, onSelect };
  }

  it('lists the chat folder, its own entries first, folders before files', async () => {
    const { ref } = renderMention('');
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(1));
    const names = (ref.current?.getDisplayFiles() ?? []).map((row) => row.name);
    expect(names.slice(0, 5)).toEqual([
      'ex-empty',
      'ex-linked',
      'ex-src',
      'ex-archive.zip',
      'readme.md',
    ]);
    expect(mocks.listFiles).toHaveBeenCalledWith('/w');
  });

  it('inserts the entry an Enter picks, as a typed query would', async () => {
    const { ref, onSelect } = renderMention('');
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(1));
    ref.current?.selectFile(0);
    expect(onSelect).toHaveBeenCalledWith('/w/ex-empty');
  });

  it('narrows the list from the first character typed', async () => {
    const { rerender } = renderMention('');
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(1));
    rerender('r');
    await waitFor(() => {
      const shown = screen.getAllByRole('option').map((row) => row.textContent ?? '');
      expect(shown.some((text) => text.startsWith('readme.md'))).toBe(true);
      expect(shown.some((text) => text.startsWith('ex-empty'))).toBe(false);
    });
  });

  it('never shows the "/" list’s rows under "@"', async () => {
    const { rerender } = renderMention('', true);
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(0));
    const commandRows = screen.getAllByRole('option').map((row) => row.textContent);
    rerender('');
    // Before the scan answers, the menu says "Searching…", not the commands it listed.
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(1));
    const fileRows = screen.getAllByRole('option').map((row) => row.textContent);
    expect(fileRows.filter((text) => commandRows.includes(text))).toEqual([]);
  });
});
