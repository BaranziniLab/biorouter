import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import {
  ModelAndProviderProvider,
  useModelAndProvider,
  type ChangeModelOptions,
} from './ModelAndProviderContext';
import ModelsBottomBar from './settings/models/bottom_bar/ModelsBottomBar';
import { __resetDisclosureStoreForTests } from './privacy/disclosureCopy';
import { usePinnedModel } from './privacy/usePinnedModel';
import { useConfirmNewChatModel } from './privacy/useConfirmNewChatModel';
import type Model from './settings/models/modelInterface';
import type { Session } from '../api/types.gen';
import type { PinnedModelView } from '../hooks/chatStreamStore';

/**
 * F3 (provider QA, 2026-09-10) — a second window's model chip was stale, and the
 * chat it started ran on a model it never showed.
 *
 * Measured on merged main `7c96d796`, both directions: change the app-wide
 * model in window 1 and window 2's chip never moved (8 s). Window 2 read
 * `gpt-5.5-2026-04-24 (Private model, UCSF)` at the instant of send; the chat it
 * created bound `claude_code`, was classified `public` — correctly — and its
 * turn went to a consumer subscription with no BAA. The privacy machinery held;
 * the label the human acted on did not.
 *
 * Root cause: `ModelAndProviderContext` read `BIOROUTER_PROVIDER` /
 * `BIOROUTER_MODEL` once, on mount, while `/agent/start` binds a new chat to
 * whatever those keys say on the daemon at that instant.
 *
 * Every test here mounts TWO `ModelAndProviderProvider` trees — two windows'
 * worth of state in one document — each rendering the real composer chip, over
 * one fake daemon whose two keys are what `/agent/start` would bind. The
 * assertion that matters throughout is the one the QA run could not make: the
 * chip a window shows equals what its next new chat would run on.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  getProviders: vi.fn(),
  refreshConfig: vi.fn(),
  setConfigProvider: vi.fn(),
  updateAgentProvider: vi.fn(),
  llamacppStatus: vi.fn(),
  llamacppWarmup: vi.fn(),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  toastWarning: vi.fn(),
}));

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  setConfigProvider: mocks.setConfigProvider,
  updateAgentProvider: mocks.updateAgentProvider,
  llamacppStatus: mocks.llamacppStatus,
  llamacppWarmup: mocks.llamacppWarmup,
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));

vi.mock('../toasts', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
  toastWarning: mocks.toastWarning,
}));

vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

// `usePrivacyTiersEnabled` too: the chip's padlock reads the master switch.
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    refreshConfig: mocks.refreshConfig,
  }),
  usePrivacyTiersEnabled: () => true,
}));

// `BaseChat` is the whole chat surface; the chip imports it for one dead context.
vi.mock('./BaseChat', () => ({ useCurrentModelInfo: () => null }));
vi.mock('./settings/models/subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: () => null,
}));
vi.mock('./settings/models/subcomponents/LeadWorkerSettings', () => ({
  LeadWorkerSettings: () => null,
}));

Object.defineProperty(window, 'appConfig', {
  writable: true,
  value: { get: () => undefined },
});

// ── The daemon ─────────────────────────────────────────────────────────────

const PRIVATE = { provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' };
const PUBLIC = { provider: 'claude_code', model: 'claude-fable-5-1' };
const CODEX = { provider: 'codex', model: 'gpt-6-astra' };

/** `config.yaml`'s two keys, as the daemon holds them. */
const daemon: { provider: string | null; model: string | null } = { ...PRIVATE };

/**
 * What the next `/agent/start` binds: `configured_new_session_provider` reads
 * exactly these two keys and nothing the renderer sends.
 */
const nextNewChatBinding = () => ({ provider: daemon.provider, model: daemon.model });

/** A write made by anything other than the renderer: the CLI, a hand edit. */
const writeOutsideTheRenderer = (next: { provider: string; model: string }) => {
  daemon.provider = next.provider;
  daemon.model = next.model;
};

const UCSF = { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] };

const providerRow = (
  name: string,
  display: string,
  tier: 'private' | 'public',
  affiliation: unknown = null
) => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: display, tier, runs_locally: false, known_models: [] },
  affiliation,
  resolved_tier: tier,
});

const PROVIDER_ROWS = [
  providerRow('versa_azure', 'Versa API Azure', 'private', UCSF),
  providerRow('claude_code', 'Claude Code', 'public'),
  providerRow('codex', 'Codex', 'public'),
];

/** The chip's accessible name, exactly as a screen reader — or QA — reads it. */
const CHIP = {
  [PRIVATE.model]: `Current model: ${PRIVATE.model} (Private model, UCSF)`,
  [PUBLIC.model]: `Current model: ${PUBLIC.model} (Public model)`,
  [CODEX.model]: `Current model: ${CODEX.model} (Public model)`,
};

/** The chip's label for whatever pair the daemon holds right now. */
const chipForDaemon = () => CHIP[nextNewChatBinding().model as string];

/** A second window speaking on the channel, as `BroadcastChannel` delivers it. */
async function announceFromAnotherWindow() {
  const other = new BroadcastChannel('biorouter:session-binding');
  other.postMessage({ kind: 'app-model-selection' });
  // Delivery is a task, not a microtask; close only once it has happened.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  other.close();
}

// ── The windows ────────────────────────────────────────────────────────────

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

const asModel = (pair: { provider: string; model: string }): Model => ({
  name: pair.model,
  provider: pair.provider,
  subtext: pair.provider,
});

/**
 * One window's Home composer: its chip, plus the two acts that matter — the
 * model switcher's commit and a send that would create a new chat.
 */
function HomeWindow({ label }: { label: string }) {
  const { changeModel } = useModelAndProvider();
  const confirmNewChatModel = useConfirmNewChatModel();
  const [sendResult, setSendResult] = useState('idle');
  return (
    <section aria-label={label}>
      <ModelsBottomBar sessionId={null} dropdownRef={dropdownRef} setView={vi.fn()} alerts={[]} />
      <button type="button" onClick={() => void changeModel(null, asModel(PUBLIC))}>
        Switch new chats to Claude Code
      </button>
      <button type="button" onClick={() => void changeModel(null, asModel(PRIVATE))}>
        Switch new chats to Versa
      </button>
      <button
        type="button"
        onClick={() => void confirmNewChatModel().then((ok) => setSendResult(String(ok)))}
      >
        Send
      </button>
      <output data-testid={`${label}-send`}>{sendResult}</output>
    </section>
  );
}

/** One window showing an existing chat, with the switcher committing for it. */
function ChatWindow({
  label,
  session,
  pin,
  switchOptions,
}: {
  label: string;
  session: Session;
  pin?: PinnedModelView;
  switchOptions?: ChangeModelOptions;
}) {
  const { changeModel } = useModelAndProvider();
  const { effectiveModel } = usePinnedModel(session, pin);
  return (
    <section aria-label={label}>
      <ModelsBottomBar
        sessionId={session.id}
        privacyTier={session.privacy_tier ?? undefined}
        effectiveModel={effectiveModel}
        dropdownRef={dropdownRef}
        setView={vi.fn()}
        alerts={[]}
      />
      <button
        type="button"
        onClick={() => void changeModel(session.id, asModel(CODEX), switchOptions)}
      >
        Switch this chat to Codex
      </button>
    </section>
  );
}

const inWindow = (label: string) => within(screen.getByRole('region', { name: label }));

const chipIn = (label: string) =>
  inWindow(label).getByRole('button', { name: /^Current model:/ }) as HTMLElement;

async function expectChip(label: string, name: string) {
  await waitFor(() => expect(chipIn(label)).toHaveAccessibleName(name));
}

function renderTwoHomeWindows() {
  return render(
    <>
      <ModelAndProviderProvider>
        <HomeWindow label="window 1" />
      </ModelAndProviderProvider>
      <ModelAndProviderProvider>
        <HomeWindow label="window 2" />
      </ModelAndProviderProvider>
    </>
  );
}

const session = (overrides: Partial<Session>): Session =>
  ({
    id: 'chat-a',
    working_dir: '/tmp',
    name: 'A chat',
    message_count: 3,
    privacy_tier: 'private',
    ...overrides,
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  Object.assign(daemon, PRIVATE);
  mocks.read.mockImplementation(async (key: string) => {
    if (key === 'BIOROUTER_MODEL') return daemon.model;
    if (key === 'BIOROUTER_PROVIDER') return daemon.provider;
    return null;
  });
  mocks.getProviders.mockResolvedValue(PROVIDER_ROWS);
  mocks.refreshConfig.mockResolvedValue(undefined);
  // `/config/set_provider`, as the daemon applies it.
  mocks.setConfigProvider.mockImplementation(
    async ({ body }: { body: { provider: string; model: string } }) => {
      daemon.provider = body.provider;
      daemon.model = body.model;
      return { data: null };
    }
  );
  mocks.updateAgentProvider.mockResolvedValue({ data: '' });
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: {
      title_template: '{provider} is not hosted by your institution.',
      long: 'LONG',
      short: 'SHORT',
      acknowledged: true,
    },
  });
});

describe('F3 — the app-wide selection reaches every window', () => {
  /**
   * The brief's first test, as stated: two consumers, the broadcast, and the
   * second one's model, provider and privacy label updating without a remount.
   * The announcement arrives the way another BrowserWindow's does — on the
   * channel, from a different `BroadcastChannel` object.
   *
   * Fails on 7c96d796: nothing listens for it, and window 2 keeps
   * `gpt-5.5-2026-04-24 (Private model, UCSF)` for as long as it lives.
   */
  it("updates a second window's chip — model, provider and privacy — without a remount", async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);
    const before = chipIn('window 2');

    writeOutsideTheRenderer(PUBLIC);
    await announceFromAnotherWindow();

    await expectChip('window 2', CHIP[PUBLIC.model]);
    expect(chipIn('window 2')).not.toHaveAccessibleName(/Private model/);
    // The same element, re-rendered — not a fresh mount that re-read on mount.
    expect(chipIn('window 2')).toBe(before);
  });

  /**
   * The brief's second test: a stale label cannot survive a change, in either
   * direction. "The value the next `/agent/start` would bind" is the fake
   * daemon's pair, which is exactly what `configured_new_session_provider`
   * reads.
   */
  it('states what the next new chat would bind, after a switch in either direction', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    // Private → public. The direction QA measured going to a no-BAA plan.
    fireEvent.click(inWindow('window 1').getByRole('button', { name: /to Claude Code/ }));
    await waitFor(() => expect(nextNewChatBinding()).toEqual(PUBLIC));
    await expectChip('window 2', chipForDaemon());
    expect(chipIn('window 2')).toHaveAccessibleName(CHIP[PUBLIC.model]);
    expect(chipIn('window 1')).toHaveAccessibleName(CHIP[PUBLIC.model]);

    // And back.
    fireEvent.click(inWindow('window 1').getByRole('button', { name: /to Versa/ }));
    await waitFor(() => expect(nextNewChatBinding()).toEqual(PRIVATE));
    await expectChip('window 2', chipForDaemon());
    expect(chipIn('window 2')).toHaveAccessibleName(CHIP[PRIVATE.model]);
  });

  /**
   * ⚠ The nudge is not a payload, and this is why: two windows' writes can be
   * announced in the opposite order from the one in which they landed. A
   * receiver that applied values would end on the last MESSAGE; one that
   * re-reads ends on the last WRITE — the one `/agent/start` binds.
   */
  it('ends on the write that landed last, not the announcement that arrived last', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    // Two writes land — Codex, then Claude Code — and both are announced.
    writeOutsideTheRenderer(CODEX);
    writeOutsideTheRenderer(PUBLIC);
    await announceFromAnotherWindow();
    await announceFromAnotherWindow();

    await expectChip('window 2', CHIP[PUBLIC.model]);
  });
});

describe('F3 — reads are ordered by when they were issued', () => {
  /**
   * A read that left before a newer statement was published must not come back
   * afterwards and restore the model the newer one replaced. Window 2 re-reads
   * twice; the first answer is held until after the second has landed.
   */
  it('does not let a slower, older read overwrite a newer one', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    let releaseOld: (value: string) => void = () => {};
    const heldModel = new Promise<string>((resolve) => {
      releaseOld = resolve;
    });
    // While `holding`, every read answers with the OLD pair, and slowly.
    let holding = true;
    mocks.read.mockImplementation(async (key: string) => {
      if (holding && key === 'BIOROUTER_MODEL') return heldModel;
      if (holding && key === 'BIOROUTER_PROVIDER') return PRIVATE.provider;
      if (key === 'BIOROUTER_MODEL') return daemon.model;
      if (key === 'BIOROUTER_PROVIDER') return daemon.provider;
      return null;
    });

    await announceFromAnotherWindow();
    holding = false;
    writeOutsideTheRenderer(PUBLIC);
    await announceFromAnotherWindow();
    await expectChip('window 2', CHIP[PUBLIC.model]);

    // The first read finally answers — with the pair the second one replaced.
    await act(async () => {
      releaseOld(PRIVATE.model);
      await heldModel;
    });
    expect(chipIn('window 2')).toHaveAccessibleName(CHIP[PUBLIC.model]);
  });

  /**
   * ⚠ The other half of the rule: a NEWER read that fails publishes nothing, so
   * it must not condemn an older one that succeeded
   * (`renderer-testing-traps.md`, "Newest issued is the wrong rule").
   */
  it('keeps an older read that succeeded when a newer one fails', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    writeOutsideTheRenderer(PUBLIC);
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    // The first nudge's reads are slow and right; the second's are fast and
    // failed — the generated client resolves a dead daemon with no body.
    let phase: 'slow' | 'failing' = 'slow';
    mocks.read.mockImplementation(async (key: string) => {
      if (phase === 'failing') return undefined;
      await gate;
      if (key === 'BIOROUTER_MODEL') return daemon.model;
      if (key === 'BIOROUTER_PROVIDER') return daemon.provider;
      return null;
    });

    await announceFromAnotherWindow();
    phase = 'failing';
    await announceFromAnotherWindow();
    await act(async () => {
      release();
      await gate;
    });

    await expectChip('window 2', CHIP[PUBLIC.model]);
  });

  /** A failed read is not evidence that nothing is configured. */
  it('keeps the label it has when a re-read fails, rather than erasing it', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    mocks.read.mockResolvedValue(undefined);
    await announceFromAnotherWindow();

    expect(chipIn('window 2')).toHaveAccessibleName(CHIP[PRIVATE.model]);
    expect(screen.queryByRole('button', { name: 'Choose a model' })).toBeNull();
  });
});

describe('F3 — a write nothing announces', () => {
  /**
   * `biorouter configure` in a terminal writes `config.yaml`, the daemon's
   * cache is keyed on the file's stamp, and nothing tells any window. Coming
   * back to the window is when the user acts, so that is when it re-reads.
   */
  it('re-reads when the window regains focus', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    writeOutsideTheRenderer(PUBLIC);
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });

    await expectChip('window 2', CHIP[PUBLIC.model]);
  });

  it('re-reads when the window becomes visible again', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    writeOutsideTheRenderer(CODEX);
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });

    await expectChip('window 2', CHIP[CODEX.model]);
  });

  /**
   * A terminal docked INSIDE the window never takes its focus, so a send can
   * still be the first thing to notice. It is refused, the fresh model is put
   * on screen, and the toast says what changed in words — including the tier,
   * which is the reason any of this matters.
   */
  it('refuses a send made on a stale chip, and shows the model it would have used', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    writeOutsideTheRenderer(PUBLIC);
    fireEvent.click(inWindow('window 2').getByRole('button', { name: 'Send' }));

    await waitFor(() => expect(screen.getByTestId('window 2-send')).toHaveTextContent('false'));
    await expectChip('window 2', CHIP[PUBLIC.model]);
    expect(mocks.toastWarning).toHaveBeenCalledWith(
      expect.objectContaining({
        title: 'Message not sent',
        msg: expect.stringContaining(
          `New chats now start on ${PUBLIC.model} (Claude Code, a public model), not ${PRIVATE.model}`
        ),
      })
    );
  });

  it('lets a send through when the chip already states what a new chat binds', async () => {
    renderTwoHomeWindows();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    fireEvent.click(inWindow('window 2').getByRole('button', { name: 'Send' }));

    await waitFor(() => expect(screen.getByTestId('window 2-send')).toHaveTextContent('true'));
    expect(mocks.toastWarning).not.toHaveBeenCalled();
  });
});

describe('F3 / privacy-tiers P4 — a switch made in a chat is about that chat', () => {
  function renderChatAndHome(switchOptions?: ChangeModelOptions) {
    return render(
      <>
        <ModelAndProviderProvider>
          <ChatWindow
            label="window 1"
            session={session({
              id: 'chat-a',
              privacy_tier: 'public',
              provider_name: PUBLIC.provider,
              model_config: { model_name: PUBLIC.model } as Session['model_config'],
            })}
            switchOptions={switchOptions}
          />
        </ModelAndProviderProvider>
        <ModelAndProviderProvider>
          <HomeWindow label="window 2" />
        </ModelAndProviderProvider>
      </>
    );
  }

  /**
   * QA F bound Claude Code in one chat for one check, and the next chat it
   * opened came up public: the switch had silently moved the app-wide default.
   * Unticked, it moves the chat and nothing else — so the other window's
   * new-chat chip does not move, and it is RIGHT not to.
   */
  it('leaves the model new chats start on alone, in every window', async () => {
    renderChatAndHome();
    await expectChip('window 2', CHIP[PRIVATE.model]);

    fireEvent.click(inWindow('window 1').getByRole('button', { name: /this chat to Codex/ }));

    await waitFor(() => expect(mocks.updateAgentProvider).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalled());
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
    expect(nextNewChatBinding()).toEqual(PRIVATE);
    expect(chipIn('window 2')).toHaveAccessibleName(chipForDaemon());
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      expect.objectContaining({
        msg: `This chat now uses ${CODEX.model} from ${CODEX.provider}. Other chats, and new ones, are unchanged.`,
      })
    );
  });

  it('moves every window when the user asks for new chats too', async () => {
    renderChatAndHome({ alsoForNewChats: true });
    await expectChip('window 2', CHIP[PRIVATE.model]);

    fireEvent.click(inWindow('window 1').getByRole('button', { name: /this chat to Codex/ }));

    await waitFor(() => expect(nextNewChatBinding()).toEqual(CODEX));
    await expectChip('window 2', chipForDaemon());
    expect(chipIn('window 2')).toHaveAccessibleName(CHIP[CODEX.model]);
  });
});

describe('F3 — the pin still outranks a stale row', () => {
  /**
   * A chat's composer states the chat's own binding, and `chatBinding` prefers
   * the turn-reported pin over the cached row (`privacy/pinnedModel.ts`, and the
   * PIN note in `utils/sessionBindingSync.ts`).
   * An app-wide change crossing windows must not disturb that: the chat below
   * last RAN on Versa (the pin), its cached row still names Codex, and the
   * selection moves to Claude Code. Its chip names Versa before and after.
   */
  it('keeps naming the pinned model when the app-wide selection moves under it', async () => {
    const stale = session({
      id: 'chat-b',
      privacy_tier: 'private',
      provider_name: CODEX.provider,
      model_config: { model_name: CODEX.model } as Session['model_config'],
    });
    render(
      <>
        <ModelAndProviderProvider>
          <HomeWindow label="window 1" />
        </ModelAndProviderProvider>
        <ModelAndProviderProvider>
          <ChatWindow label="window 2" session={stale} pin={PRIVATE} />
        </ModelAndProviderProvider>
      </>
    );
    await expectChip(
      'window 2',
      `${CHIP[PRIVATE.model]}. Private chat. Biorouter only lets a private model open it.`
    );

    fireEvent.click(inWindow('window 1').getByRole('button', { name: /to Claude Code/ }));
    await waitFor(() => expect(nextNewChatBinding()).toEqual(PUBLIC));
    await expectChip('window 1', CHIP[PUBLIC.model]);

    expect(chipIn('window 2')).toHaveAccessibleName(
      `${CHIP[PRIVATE.model]}. Private chat. Biorouter only lets a private model open it.`
    );
    expect(chipIn('window 2')).not.toHaveAccessibleName(new RegExp(CODEX.model));
    expect(chipIn('window 2')).not.toHaveAccessibleName(new RegExp(PUBLIC.model));
  });
});
