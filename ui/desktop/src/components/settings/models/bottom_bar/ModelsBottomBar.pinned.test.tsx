import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar, {
  CHAT_KEEPS_ITS_MODEL_NOTE,
  NEW_CHATS_MODEL_HEADING,
  NEW_CHATS_MODEL_NOTE,
} from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

/**
 * Issue #56 / F2 — the chip states what runs in THIS chat.
 *
 * The measured defect: Settings → Models → Claude Code / claude-opus-5, then
 * send into a chat whose `privacy_tier` is `private`. The turn was served by
 * Versa — `restore_provider_from_session` binds the session row's own provider
 * — while this chip read `claude-opus-5`, the model that was not used, and the
 * gauge beside it sized itself to Claude's 1M window.
 */
const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => ''),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));

// The GLOBAL selection throughout this file: the public model the user picked.
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'claude-opus-5',
    currentProvider: 'claude_code',
    getCurrentModelAndProviderForDisplay: async () => ({
      model: 'claude-opus-5',
      provider: 'Claude Code',
    }),
    getCurrentModelDisplayName: async () => 'claude-opus-5',
    getCurrentProviderDisplayName: async () => 'Claude Code',
  }),
}));

vi.mock('../../../BaseChat', () => ({ useCurrentModelInfo: () => null }));
vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

const PINNED = { provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' };

const providerEntry = (name: string, display: string, tier: 'private' | 'public') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: display, tier, runs_locally: false },
  affiliation: null,
  resolved_tier: tier,
});

function renderBar(effectiveModel?: { provider: string; model: string }) {
  return render(
    <ModelsBottomBar
      sessionId="s1"
      privacyTier="private"
      effectiveModel={effectiveModel}
      dropdownRef={dropdownRef}
      setView={vi.fn()}
      alerts={[]}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([
    providerEntry('versa_azure', 'Versa API Azure', 'private'),
    providerEntry('claude_code', 'Claude Code', 'public'),
  ]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: {
      title_template: '{provider} is not hosted by your institution.',
      long: 'LONG',
      short: 'SHORT',
      acknowledged: true,
    },
  });
});

describe('a chat bound to something other than the app-wide selection', () => {
  it('names the model that actually runs here, not the app-wide selection', async () => {
    renderBar(PINNED);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /Current model: gpt-5\.5-2026-04-24/ })
      ).toBeInTheDocument()
    );
    expect(screen.queryByText('claude-opus-5')).toBeNull();
  });

  /**
   * Not just the name. The padlock and the tier word are read off the BOUND
   * provider's catalog row; reading the global provider's row would put a
   * "Public model" label under a private model's name — the precise
   * misattribution the chip's own comments were written against.
   */
  it('states the bound provider’s tier, not the selected one’s', async () => {
    renderBar(PINNED);
    const trigger = await screen.findByRole('button', { name: /Current model:/ });
    await waitFor(() => expect(trigger).toHaveAccessibleName(/Private model/));
    expect(trigger).not.toHaveAccessibleName(/Public model/);
  });

  it('names the chat’s own binding in the dropdown header, by display name', async () => {
    renderBar(PINNED);
    await screen.findByRole('button', { name: /Current model:/ });
    // `pointerDown`, not `click`: Radix's dropdown trigger opens on the pointer
    // event, exactly as the sibling suite in this directory does it.
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
    const header = await screen.findByText('Current model');
    await waitFor(() =>
      expect(header.parentElement).toHaveTextContent('gpt-5.5-2026-04-24 · Versa API Azure')
    );
  });

  it('leaves a chat with no binding of its own showing the app-wide selection', async () => {
    renderBar(undefined);
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: /Current model: claude-opus-5/ })
      ).toBeInTheDocument()
    );
    expect(screen.queryByText('gpt-5.5-2026-04-24')).toBeNull();
  });

  /**
   * Round 3 / N1. The chip and gauge now state the chat's own binding for EVERY
   * chat that has one, so they disagree with the app-wide selection in every
   * chat older than the user's last model switch — the ordinary case. That
   * raises a question the chip alone cannot answer ("did my switch fail?"), and
   * this is where it is answered: inside the dropdown, which is what a reader
   * opens to ask the chip what model this chat is on.
   *
   * ⚠ Deliberately NOT a note above the composer. That surface belongs to the
   * privacy sentence, which is earned by a barrier and is rare; a standing
   * banner for the ordinary case would be near-permanent chrome restating what
   * the control beside it already says.
   */
  it('explains in the dropdown why the chip may differ from the app-wide choice', async () => {
    renderBar(PINNED);
    await screen.findByRole('button', { name: /Current model:/ });
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });

    const note = await screen.findByTestId('chat-binding-note');
    expect(note).toHaveTextContent(CHAT_KEEPS_ITS_MODEL_NOTE);
    // It names the mechanism, never privacy: this line appears on public chats
    // too, where privacy is not the cause.
    expect(note.textContent).not.toMatch(/private/i);
  });

  it('says nothing when the chat runs on exactly what is selected', async () => {
    renderBar(undefined);
    await screen.findByRole('button', { name: /Current model:/ });
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });

    await screen.findByText('Current model');
    expect(screen.queryByTestId('chat-binding-note')).toBeNull();
  });
});

/**
 * F3 — where there is no chat yet (Home, a chat not started), the chip names
 * the APP-WIDE selection, and a switch from it changes that selection for every
 * window. The dropdown says whose model it is and how far a change reaches,
 * beside the control that makes the change.
 */
describe('the chip where there is no chat yet', () => {
  const renderSessionless = () =>
    render(
      <ModelsBottomBar sessionId={null} dropdownRef={dropdownRef} setView={vi.fn()} alerts={[]} />
    );

  const openDropdown = async () => {
    await screen.findByRole('button', { name: /Current model:/ });
    fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
  };

  it('heads its dropdown as the model for new chats, reaching every window', async () => {
    renderSessionless();
    await openDropdown();

    expect(await screen.findByText(NEW_CHATS_MODEL_HEADING)).toBeInTheDocument();
    expect(screen.getByTestId('new-chats-model-note')).toHaveTextContent(NEW_CHATS_MODEL_NOTE);
    expect(screen.queryByText('Current model')).toBeNull();
  });

  it('keeps "Current model", and no such line, in a chat', async () => {
    renderBar(undefined);
    await openDropdown();

    expect(await screen.findByText('Current model')).toBeInTheDocument();
    expect(screen.queryByTestId('new-chats-model-note')).toBeNull();
  });
});
