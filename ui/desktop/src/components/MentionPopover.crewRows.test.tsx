import { render, screen, waitFor } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import MentionPopover, {
  CREW_EXTENSION_DESCRIPTION,
  type DisplayItemWithMatch,
} from './MentionPopover';
import { reachGatedGetActive, USER_ACTION_KEY } from '../test/reachGate';

const mocks = vi.hoisted(() => ({
  getActive: vi.fn(),
  listBases: vi.fn(),
  getSlashCommands: vi.fn(),
  getSessionExtensions: vi.fn(),
  // ONE array, as the real context's state is (see MentionPopover.privateChat.test.tsx).
  extensionsList: [] as never[],
}));

vi.mock('../api', () => ({
  getActive: mocks.getActive,
  listBases: mocks.listBases,
  getSlashCommands: mocks.getSlashCommands,
  getSessionExtensions: mocks.getSessionExtensions,
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ extensionsList: mocks.extensionsList }),
}));

vi.mock('./skills/useSkillCatalog', () => ({
  fetchSkillCatalog: vi.fn(async () => ({ generation: 0, roots: [], skills: [], bundles: [] })),
  pickerBundles: () => [],
  standaloneSkills: () => [],
}));

const CHAT = 'chat-1';

/** The Crew platform extension as the daemon lists it for a chat that has it on. */
const crewExtension = {
  type: 'platform',
  name: 'crew',
  display_name: 'Crew',
  description: 'Use saved Crew connections and human-approved channel context and agent posting',
  bundled: true,
  available_tools: [],
};

type Handle = {
  getDisplayFiles: () => DisplayItemWithMatch[];
  selectFile: (index: number) => void;
};

function renderPalette(query: string) {
  const ref = createRef<Handle>();
  const view = render(
    <MentionPopover
      ref={ref}
      isOpen
      isSlashCommand
      query={query}
      sessionId={CHAT}
      workingDir="/w"
      position={{ x: 0, y: 400 }}
      selectedIndex={0}
      onSelectedIndexChange={() => {}}
      onSelect={() => {}}
      onClose={() => {}}
    />
  );
  return { ref, view };
}

/**
 * Q2-71 (live QA round 2): "/cr" listed two rows that both read as crew — the `/crew` command and
 * the Crew extension, described as "Use saved Crew connections and human-approved channel
 * context…" — and people asked which one. The command comes first for any query it starts with.
 *
 * Q3-31 (live QA round 3): the second row was still there, and still confusing. Any list that shows
 * the `/crew` command leaves the extension's row out. The extension stays reachable: a query aimed
 * at extensions never matches the command, so its list shows the row, labelled as the tools.
 */
describe('the / palette’s Crew rows', () => {
  let savedElectron: unknown;

  beforeEach(() => {
    vi.clearAllMocks();
    savedElectron = (window as { electron?: unknown }).electron;
    Object.assign(window, {
      electron: { getUserActionKey: vi.fn(async () => USER_ACTION_KEY) },
    });
    mocks.getSlashCommands.mockResolvedValue({
      data: {
        commands: [
          { command: 'compact', help: 'Compact the conversation', command_type: 'Builtin' },
          { command: 'crispr-review', help: 'Review a CRISPR screen', command_type: 'Workflow' },
        ],
      },
    });
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [crewExtension] } });
    mocks.listBases.mockResolvedValue({ data: [] });
    mocks.getActive.mockImplementation(
      reachGatedGetActive([], () => ({ kb_ids: [], primary_kb: null, hidden_kbs: [] }))
    );
  });

  afterEach(() => {
    Object.assign(window, { electron: savedElectron });
  });

  const isCrewExtension = (item: DisplayItemWithMatch) =>
    item.itemType === 'Extension' && item.relativePath === 'crew';

  it.each(['cr', 'cre', 'crew', 'CR'])(
    'lists only the /crew command for “%s”, first, and not the extension beside it',
    async (query) => {
      const { ref } = renderPalette(query);
      await waitFor(() =>
        expect(
          ref.current
            ?.getDisplayFiles()
            .some((item) => item.itemType === 'Builtin' && item.name === 'crew')
        ).toBe(true)
      );
      // Everything has loaded: the other rows the list holds are there.
      await waitFor(() =>
        expect(ref.current?.getDisplayFiles().some((item) => item.itemType === 'Extension')).toBe(
          true
        )
      );
      const items = ref.current?.getDisplayFiles() ?? [];
      expect(items[0]).toMatchObject({ itemType: 'Builtin', name: 'crew' });
      expect(
        items.filter((item) => item.itemType === 'Builtin' && item.name === 'crew')
      ).toHaveLength(1);
      expect(items.some(isCrewExtension)).toBe(false);
      expect(screen.queryByText(CREW_EXTENSION_DESCRIPTION)).toBeNull();
    }
  );

  it('leaves the extension out of the whole list too, where /crew is listed', async () => {
    const { ref } = renderPalette('');
    await waitFor(() =>
      expect(ref.current?.getDisplayFiles().some((item) => item.itemType === 'Extension')).toBe(
        true
      )
    );
    const items = ref.current?.getDisplayFiles() ?? [];
    expect(items.some((item) => item.itemType === 'Builtin' && item.name === 'crew')).toBe(true);
    expect(items.some(isCrewExtension)).toBe(false);
  });

  it.each(['ext', 'ext:cr', 'ext:crew'])(
    'still reaches the extension for “%s”, labelled as the advanced Crew tools',
    async (query) => {
      const { ref } = renderPalette(query);
      await waitFor(() => expect(ref.current?.getDisplayFiles().some(isCrewExtension)).toBe(true));
      const items = ref.current?.getDisplayFiles() ?? [];
      expect(items.some((item) => item.itemType === 'Builtin' && item.name === 'crew')).toBe(false);
      expect(await screen.findByText(CREW_EXTENSION_DESCRIPTION)).toBeInTheDocument();
      expect(CREW_EXTENSION_DESCRIPTION).toBe('Crew tools extension (advanced)');
      expect(screen.queryByText(/human-approved channel context/)).toBeNull();
    }
  );

  it('says what the /crew command does', async () => {
    renderPalette('cr');
    expect(await screen.findByText('Connect this chat to a Crew channel')).toBeInTheDocument();
  });

  it('leaves every other row’s order to the match', async () => {
    const { ref } = renderPalette('co');
    await waitFor(() => expect(ref.current?.getDisplayFiles().length).toBeGreaterThan(0));
    expect(ref.current?.getDisplayFiles()[0]).toMatchObject({ name: 'compact' });
  });
});
