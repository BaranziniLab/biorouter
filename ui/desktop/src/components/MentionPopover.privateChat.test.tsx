import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import MentionPopover from './MentionPopover';
import { reachGatedGetActive, USER_ACTION_KEY } from '../test/reachGate';

const mocks = vi.hoisted(() => ({
  getActive: vi.fn(),
  listBases: vi.fn(),
  getSlashCommands: vi.fn(),
  getSessionExtensions: vi.fn(),
  // ONE array, as the real context's state is: a fresh `[]` per render
  // recreates `loadReferenceItems` every render, and the palette reloads itself
  // in a loop that detaches every row it has just drawn.
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

const PRIVATE_CHAT = 'chat-private';

function base(id: string, name: string) {
  return { id, name, color: '#cf6d47', created_at: '', schema_version: 1, tier: 'public' };
}

function renderPalette() {
  return render(
    <MentionPopover
      isOpen
      isSlashCommand
      query="kb"
      sessionId={PRIVATE_CHAT}
      workingDir="/w"
      position={{ x: 0, y: 400 }}
      selectedIndex={0}
      onSelectedIndexChange={() => {}}
      onSelect={() => {}}
      onClose={() => {}}
    />
  );
}

/**
 * Issue #56 Task 58: the `/` palette's knowledge-base rows come from the chat's
 * selection, and `GET /knowledge/active` naming a PRIVATE chat is on the
 * daemon's reach gate. Measured in the running desktop app on 2026-09-11: the
 * read went out with no proof, was refused, and the palette fell back to "no
 * base is hidden, none is primary" — offering a base the chat had hidden as
 * "Knowledge base in this chat", and naming no primary at all.
 */
describe('the / palette in a private chat', () => {
  let savedElectron: unknown;
  let savedScrollIntoView: PropertyDescriptor | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    savedElectron = (window as { electron?: unknown }).electron;
    Object.assign(window, {
      electron: { getUserActionKey: vi.fn(async () => USER_ACTION_KEY) },
    });
    // jsdom has no `scrollIntoView`, and the palette scrolls its selected row
    // into view on every render — an effect that throws unmounts the palette.
    savedScrollIntoView = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollIntoView');
    Object.defineProperty(Element.prototype, 'scrollIntoView', {
      configurable: true,
      writable: true,
      value: vi.fn(),
    });
    mocks.getSlashCommands.mockResolvedValue({ data: { commands: [] } });
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } });
    mocks.listBases.mockResolvedValue({
      data: [
        base('soul', 'Soul'),
        base('lab-notes', 'Lab notes'),
        base('grant-drafts', 'Grant drafts'),
      ],
    });
    mocks.getActive.mockImplementation(
      reachGatedGetActive([PRIVATE_CHAT], (sessionId) =>
        sessionId
          ? {
              kb_ids: ['lab-notes', 'soul'],
              primary_kb: 'lab-notes',
              active_kb: 'lab-notes',
              hidden_kbs: ['grant-drafts'],
            }
          : { kb_ids: ['grant-drafts', 'lab-notes', 'soul'], primary_kb: 'soul', hidden_kbs: [] }
      )
    );
  });

  afterEach(() => {
    Object.assign(window, { electron: savedElectron });
    if (savedScrollIntoView) {
      Object.defineProperty(Element.prototype, 'scrollIntoView', savedScrollIntoView);
    } else {
      delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });

  it("offers this chat's knowledge bases, not every base, and names its primary", async () => {
    renderPalette();

    expect(await screen.findByText('kb:Lab notes')).toBeInTheDocument();
    expect(screen.getByText('Primary knowledge base · lab-notes')).toBeInTheDocument();
    expect(screen.getByText('kb:Soul')).toBeInTheDocument();
    expect(screen.queryByText('kb:Grant drafts')).not.toBeInTheDocument();
    expect(mocks.getActive).toHaveBeenCalledWith(
      expect.objectContaining({
        query: { session_id: PRIVATE_CHAT },
        headers: { 'X-User-Action': USER_ACTION_KEY },
      })
    );
  });
});
