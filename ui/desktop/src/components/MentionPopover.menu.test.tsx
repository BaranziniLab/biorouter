import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import MentionPopover from './MentionPopover';
import { COMPOSER_COPY } from './composer/copy';

/**
 * The mention and slash-command menu wears the app's one menu recipe (spec 3.7):
 * rows in the shared 32px row, no "N items found" header, a spinner with
 * "Searching…" while it loads, and the modal-dropdown layer token instead of a
 * literal z-index.
 */
const mocks = vi.hoisted(() => ({
  getSlashCommands: vi.fn(),
  listBases: vi.fn(),
  getSessionExtensions: vi.fn(),
  extensionsList: [] as never[],
}));

vi.mock('../api', () => ({
  getActive: vi.fn(async () => ({ data: null })),
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

function renderMenu({
  query = '',
  isSlashCommand = true,
  selectedIndex = 0,
}: { query?: string; isSlashCommand?: boolean; selectedIndex?: number } = {}) {
  return render(
    <MentionPopover
      isOpen
      isSlashCommand={isSlashCommand}
      query={query}
      sessionId={null}
      workingDir="/w"
      position={{ x: 0, y: 400 }}
      selectedIndex={selectedIndex}
      onSelectedIndexChange={() => {}}
      onSelect={vi.fn()}
      onClose={() => {}}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, { electron: { getUserActionKey: vi.fn(async () => null) } });
  mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } });
  mocks.listBases.mockResolvedValue({ data: [] });
  mocks.getSlashCommands.mockResolvedValue({
    data: {
      commands: [
        { command: 'compact', help: 'Compact the chat', command_type: 'Builtin' },
        { command: 'clear', help: 'Clear the chat', command_type: 'Builtin' },
      ],
    },
  });
});

describe('the mention menu', () => {
  it('says "Searching…" with a spinner while it loads', async () => {
    mocks.getSlashCommands.mockReturnValue(new Promise(() => {}));
    renderMenu();
    expect(await screen.findByText(COMPOSER_COPY.mention.searching)).toBeInTheDocument();
    expect(document.querySelector('.br-spinner')).not.toBeNull();
  });

  it('lists rows in the shared menu recipe, with no count header', async () => {
    renderMenu({ selectedIndex: 1 });
    const options = await screen.findAllByRole('option');
    expect(options.length).toBeGreaterThan(1);
    expect(screen.queryByText(/items? found/)).toBeNull();
    for (const option of options) {
      expect(option.className).toContain('min-h-control-md');
      expect(option.className).toContain('text-secondary');
    }
    expect(options[1]).toHaveAttribute('aria-selected', 'true');
    expect(options[1]).toHaveAttribute('data-selected');
    expect(options[0]).not.toHaveAttribute('data-selected');
  });

  it('sits on the modal-dropdown layer, not a literal z-index', async () => {
    renderMenu();
    const menu = await screen.findByTestId('mention-popover');
    expect(menu.className).toContain('br-mention-menu');
    expect(menu.className).not.toMatch(/z-\[\d+\]/);
  });

  it('uses no off-scale type', async () => {
    renderMenu();
    await screen.findAllByRole('option');
    expect(document.body.innerHTML).not.toMatch(/text-\[1[01]px\]/);
  });

  it('draws nothing for an "@" with nothing typed after it', async () => {
    renderMenu({ isSlashCommand: false, query: '' });
    await waitFor(() => expect(screen.queryByTestId('mention-popover')).toBeNull());
  });
});
