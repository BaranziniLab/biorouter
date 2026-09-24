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
 * context…" — and people asked which one. The command comes first for any query it starts with,
 * and the extension's row says it is the advanced tools.
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

  it.each(['cr', 'cre', 'crew', 'CR'])(
    'puts the /crew command first for “%s”, and the extension below it',
    async (query) => {
      const { ref } = renderPalette(query);
      await waitFor(() =>
        expect(ref.current?.getDisplayFiles().some((item) => item.itemType === 'Extension')).toBe(
          true
        )
      );
      const items = ref.current?.getDisplayFiles() ?? [];
      expect(items[0]).toMatchObject({ itemType: 'Builtin', name: 'crew' });
      const extension = items.findIndex(
        (item) => item.itemType === 'Extension' && item.relativePath === 'crew'
      );
      expect(extension).toBeGreaterThan(0);
      expect(
        items.filter((item) => item.itemType === 'Builtin' && item.name === 'crew')
      ).toHaveLength(1);
    }
  );

  it('says the extension’s row is the advanced Crew tools', async () => {
    renderPalette('cr');
    expect(await screen.findByText(CREW_EXTENSION_DESCRIPTION)).toBeInTheDocument();
    expect(CREW_EXTENSION_DESCRIPTION).toBe('Crew tools extension (advanced)');
    expect(screen.getByText('Connect this chat to a Crew channel')).toBeInTheDocument();
    expect(screen.queryByText(/human-approved channel context/)).toBeNull();
  });

  it('leaves every other row’s order to the match', async () => {
    const { ref } = renderPalette('co');
    await waitFor(() => expect(ref.current?.getDisplayFiles().length).toBeGreaterThan(0));
    expect(ref.current?.getDisplayFiles()[0]).toMatchObject({ name: 'compact' });
  });
});
