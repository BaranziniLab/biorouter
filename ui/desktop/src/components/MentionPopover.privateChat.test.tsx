import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
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

  beforeEach(() => {
    vi.clearAllMocks();
    savedElectron = (window as { electron?: unknown }).electron;
    Object.assign(window, {
      electron: { getUserActionKey: vi.fn(async () => USER_ACTION_KEY) },
    });
    // The palette scrolls its selected row into view on every render, and jsdom
    // implements no `scrollIntoView`. The polyfill is installed ONCE in
    // src/test/setup.ts and deliberately not re-installed here: that scroll runs
    // from a PASSIVE effect, which React can flush after this file's `afterEach`
    // has run, so a polyfill with a per-test lifetime is removed while an effect
    // that needs it is still queued. See the note in setup.ts.
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

  /**
   * With the proof attached, a read that still fails is a genuine error: a
   * surface that cannot prove the person, a dropped connection, an older
   * daemon. None of those said "no base is hidden, none is primary", and the
   * palette used to render exactly that — every base as "Knowledge base in
   * this chat", Grant drafts included, and no primary.
   *
   * It keeps offering every base, because a reference names its base by id and
   * an explicit id reaches a base whatever the chat's selection
   * (`kb_id_or_primary` in the knowledge server). What it drops is the claim.
   */
  describe('when its selection cannot be read', () => {
    let warn: MockInstance;

    beforeEach(() => {
      // The failure is reported, not swallowed; this keeps it out of the run's
      // output and lets the test say so.
      warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    });

    afterEach(() => {
      warn.mockRestore();
    });

    it.each<[string, () => void]>([
      [
        'the daemon refuses it',
        // A preload with no bridge: `userActionHeaders()` sends no proof, and
        // the gate answers the way it answers any caller that has none.
        () => Object.assign(window, { electron: {} }),
      ],
      [
        'the request fails in transit',
        () => mocks.getActive.mockResolvedValue({ error: new TypeError('Failed to fetch') }),
      ],
      [
        'the request throws',
        // Before, this took the whole palette with it: `Promise.all` rejected
        // and not one row, command or skill, was left to pick.
        () => mocks.getActive.mockRejectedValue(new SyntaxError('Unexpected end of JSON input')),
      ],
    ])('offers every base and claims none of them for the chat when %s', async (_, fail) => {
      fail();
      renderPalette();

      expect(await screen.findByText('kb:Grant drafts')).toBeInTheDocument();
      expect(screen.getByText('kb:Lab notes')).toBeInTheDocument();
      expect(screen.getByText('kb:Soul')).toBeInTheDocument();
      for (const id of ['grant-drafts', 'lab-notes', 'soul']) {
        expect(screen.getByText(`Knowledge base · ${id}`)).toBeInTheDocument();
      }
      expect(screen.queryAllByText(/Primary knowledge base/)).toHaveLength(0);
      expect(screen.queryAllByText(/in this chat/)).toHaveLength(0);
      expect(warn).toHaveBeenCalledWith('Knowledge selection not read:', expect.any(String));
    });
  });
});
