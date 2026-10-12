import * as React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionsPanel, useExtensionPicker } from './BottomMenuExtensionSelection';
import { EXTENSIONS_COPY, TOOLS_COPY } from './copy';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';
import { toastService } from '../../toasts';

/**
 * The extensions list as the Tools chip holds it (spec 3.7): the hook lives for
 * the whole time the composer does, the panel shows while its list is open. The
 * harness's button stands in for the chip and states the hook's count the way
 * the old standalone chip did, so every count assertion reads one label.
 */
function BottomMenuExtensionSelection({ sessionId }: { sessionId: string | null }) {
  const [open, setOpen] = React.useState(false);
  const picker = useExtensionPicker(sessionId, { open, onOpenRequest: () => setOpen(true) });
  return (
    <>
      <button
        type="button"
        aria-label={`Manage extensions (${picker.activeCount} added)`}
        data-answered={String(picker.answered)}
        onClick={() => setOpen((current) => !current)}
      />
      {open && <ExtensionsPanel picker={picker} />}
    </>
  );
}

/** Opens (or closes) the harness's list. */
function toggleList() {
  fireEvent.click(screen.getByLabelText(/Manage extensions/));
}

const mocks = vi.hoisted(() => ({
  overrides: new Map<string, boolean>(),
  /** Entries a test adds to the config's list, on top of the fixture below. */
  extraExtensions: [] as Array<Record<string, unknown>>,
  getSessionExtensions: vi.fn(async () => ({ data: { extensions: [] } })),
  addToAgent: vi.fn(async (): Promise<void> => undefined),
  removeFromAgent: vi.fn(async (): Promise<void> => undefined),
}));

/**
 * The fixture is the experiment, so it is designed rather than inherited.
 *
 * FOUR shipped capabilities — `autovisualiser`, `code_execution`, `chatrecall`,
 * `agent_drafter`, all in `CAPABILITY_KEYS` — and THREE user-installed
 * extensions. Two of each kind are enabled, and the two halves are enabled in
 * different numbers.
 *
 * ⚠ **Both properties are load-bearing.** A fixture of capabilities alone can
 * only ever assert `0`, which is indistinguishable from the chip being broken in
 * the direction it was already broken once (v1.89.0 P-04 read `(0 enabled)` on a
 * working chat). And a fixture where the two halves happen to agree cannot tell
 * "counted the user's extensions" from "counted everything". Here every expected
 * number is non-zero and every one of them differs from the number a chip that
 * ignored `isCapabilityExtension` would print.
 */
// The selector resolves the BOUND MODEL's tier to judge pairings (issue #56 —
// see `useBoundProviderTier`), so it reads this context and the provider
// catalogue. Neither is what this file is about: the rows below are all public
// extensions, so no pairing is refused whatever the tier says. The pairing
// states themselves live in `BottomMenuExtensionSelection.privacy.test.tsx`.
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentProvider: 'versa_azure' }),
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: async () => [
      { name: 'versa_azure', is_configured: true, resolved_tier: 'private' },
    ],
    extensionsList: [
      {
        type: 'builtin',
        name: 'autovisualiser',
        display_name: 'Auto Visualiser',
        description: 'Visualization capability',
        enabled: true,
      },
      {
        type: 'platform',
        name: 'code_execution',
        description: 'Code execution capability',
        enabled: true,
      },
      {
        type: 'platform',
        name: 'chatrecall',
        description: 'Chat recall capability',
        enabled: false,
      },
      {
        type: 'builtin',
        name: 'agent_drafter',
        description: 'Agent drafter capability',
        enabled: false,
      },
      {
        type: 'stdio',
        name: 'example',
        display_name: 'Example',
        description: 'Example extension',
        cmd: 'example',
        args: [],
        enabled: false,
      },
      {
        type: 'stdio',
        name: 'spoke',
        display_name: 'Spoke',
        description: 'User-installed extension',
        cmd: 'spoke',
        args: [],
        enabled: true,
      },
      {
        type: 'stdio',
        name: 'notetaker',
        display_name: 'Workspace',
        description: 'User-installed extension',
        cmd: 'notetaker',
        args: [],
        enabled: true,
      },
      ...mocks.extraExtensions,
    ],
  }),
}));

// Only the built-in predicate is stubbed: the NAME a row reads is the real resolver's
// (`extensionDisplayName`), so these tests see what a person sees.
vi.mock('../settings/extensions/subcomponents/ExtensionList', () => ({
  isBuiltInExtension: () => false,
}));

vi.mock('../../api', () => ({
  getSessionExtensions: mocks.getSessionExtensions,
}));

// The proof the desktop sends. Since issue #56's QA sweep (2026-09-10) the
// daemon answers a request without it as a public model — private chats and
// knowledge bases omitted or refused — so each call here must carry it.
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../settings/extensions/agent-api', () => ({
  addToAgent: mocks.addToAgent,
  removeFromAgent: mocks.removeFromAgent,
}));

vi.mock('../../store/extensionOverrides', () => ({
  setExtensionOverride: (name: string, enabled: boolean) => mocks.overrides.set(name, enabled),
  getExtensionOverrides: () => mocks.overrides,
}));

vi.mock('../../toasts', () => ({
  toastService: { success: vi.fn(), error: vi.fn() },
}));

describe('BottomMenuExtensionSelection', () => {
  beforeEach(() => {
    mocks.overrides.clear();
    mocks.extraExtensions.length = 0;
    vi.clearAllMocks();
    // `clearAllMocks` clears CALLS, not implementations, so a `mockResolvedValue`
    // set by one test would otherwise be the starting state of the next.
    mocks.getSessionExtensions.mockReset();
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } } as never);
    // ⚠ Same reason, and it bites harder here because `mockImplementationOnce`
    // queues. A test that sets one up and then FAILS before the click that would
    // consume it leaves it queued for the next test, which then consumes a
    // stale promise whose resolver closes over the dead test's variable — its
    // own `resolveEnable` stays `undefined`, the toggle chain never settles, and
    // the failure surfaces as "removeFromAgent was never called". That is
    // exactly how a single wrong expected number in the chip tests above took
    // the serialization test down with it.
    mocks.addToAgent.mockReset();
    mocks.addToAgent.mockResolvedValue(undefined as never);
    mocks.removeFromAgent.mockReset();
    mocks.removeFromAgent.mockResolvedValue(undefined as never);
  });

  it('opens this chat menu when the tool-count warning asks to manage it', async () => {
    render(<BottomMenuExtensionSelection sessionId="session-1" />);

    act(() =>
      window.dispatchEvent(
        new CustomEvent('current-chat-extensions:open', {
          detail: { sessionId: 'another-session' },
        })
      )
    );
    expect(screen.queryByPlaceholderText(EXTENSIONS_COPY.search)).not.toBeInTheDocument();

    act(() =>
      window.dispatchEvent(
        new CustomEvent('current-chat-extensions:open', {
          detail: { sessionId: 'session-1' },
        })
      )
    );

    expect(await screen.findByPlaceholderText(EXTENSIONS_COPY.search)).toBeInTheDocument();
  });

  it('keeps an immediate hub toggle when the menu closes and reopens', async () => {
    render(<BottomMenuExtensionSelection sessionId={null} />);
    const trigger = screen.getByLabelText(/Manage extensions/);
    expect(trigger).not.toHaveAttribute('title');
    fireEvent.click(trigger);

    const toggle = await screen.findByRole('option', { name: 'Example' });
    expect(toggle).toHaveAttribute('aria-selected', 'false');
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));

    toggleList();
    await waitFor(() => expect(screen.queryByRole('option')).not.toBeInTheDocument());
    fireEvent.click(trigger);

    const reopenedToggle = await screen.findByRole('option', { name: 'Example' });
    expect(reopenedToggle).toHaveAttribute('aria-selected', 'true');
    fireEvent.click(reopenedToggle);
    await waitFor(() => expect(reopenedToggle).toHaveAttribute('aria-selected', 'false'));
    expect(mocks.overrides.get('example')).toBe(false);
  });

  /**
   * The chip's number is **"extensions I added"**, not "everything this chat
   * loaded".
   *
   * Shipped capabilities come with the app, are managed in Settings → Chat →
   * Capabilities rather than in this menu, and are present in every chat — so
   * counting them makes the chip a constant plus a small number, which is not
   * what a reader takes it to mean. The exclusion uses `isCapabilityExtension`,
   * the same predicate the menu already filters its own rows by, so the chip and
   * the list it labels can never disagree about what an extension is.
   *
   * What survives from v1.89.0 P-04 is the *source*: the number comes from the
   * SESSION's extensions — what the agent actually holds, and what History's own
   * per-session column reports — not from the config-derived rows. That is why
   * this chat shows 3 rows and a chip of 2: `spoke` is listed and togglable but
   * not attached here, and `example` is attached despite being off in the config.
   */
  it('counts the chat`s user extensions and not its shipped capabilities', async () => {
    mocks.getSessionExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'builtin', name: 'autovisualiser' },
          { type: 'platform', name: 'code_execution' },
          { type: 'platform', name: 'chatrecall' },
          { type: 'stdio', name: 'example' },
          { type: 'stdio', name: 'notetaker' },
        ],
      },
    } as never);

    render(<BottomMenuExtensionSelection sessionId="session-1" />);

    // Five attached, three of them shipped capabilities → 2. Counting the whole
    // session would say 5.
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (2 added)')).toBeInTheDocument()
    );

    // …and the menu lists every user extension, attached or not, so the chip is
    // a count over the chat rather than over the rows beneath it.
    toggleList();
    expect(await screen.findAllByRole('option')).toHaveLength(3);
  });

  /**
   * The exclusion asks "is this a shipped capability", NOT "is this in the
   * config". An extension the session holds and the config cannot describe is
   * the user's own by elimination, and the safe direction for a count of the
   * user's own things is to show it — so it counts.
   */
  it('counts a session extension the config has never heard of', async () => {
    mocks.getSessionExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'builtin', name: 'autovisualiser' },
          { type: 'stdio', name: 'unlisted' },
        ],
      },
    } as never);

    render(<BottomMenuExtensionSelection sessionId="session-1" />);

    // 1, not 2 (the capability counted) and not 0 (the unknown swallowed).
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument()
    );
  });

  it('counts the config`s enabled user extensions before the session read lands', async () => {
    // The daemon applies exactly this fallback for a session with no stored
    // extension state, so the chip must not flash 0 on the way to the number —
    // and the fallback obeys the same rule as the real read.
    mocks.getSessionExtensions.mockReturnValue(new Promise(() => {}) as never);

    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    await act(async () => {
      await Promise.resolve();
    });

    // Four extensions are enabled in the fixture. Two are capabilities
    // (autovisualiser, code_execution) and do not count; two are the user's own
    // (spoke, workspace) and do.
    expect(screen.getByLabelText('Manage extensions (2 added)')).toBeInTheDocument();
  });

  /**
   * V2-R7-3: the Tools chip shows its number only once every list has answered
   * for THIS chat. The fallback above is still the count while the read is out;
   * `answered` is what tells the chip not to show it.
   */
  it('has not answered while the chat’s read is out, and has once it lands', async () => {
    let resolveRead: ((value: unknown) => void) | undefined;
    mocks.getSessionExtensions.mockReturnValue(
      new Promise((resolve) => {
        resolveRead = resolve;
      }) as never
    );
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    const chip = screen.getByLabelText(/Manage extensions/);
    expect(chip).toHaveAttribute('data-answered', 'false');

    await act(async () => {
      resolveRead?.({ data: { extensions: [{ type: 'stdio', name: 'example' }] } });
      await Promise.resolve();
    });
    expect(chip).toHaveAttribute('data-answered', 'true');
    expect(chip).toHaveAccessibleName('Manage extensions (1 added)');
  });

  it('has answered when the chat’s read fails, so the chip is not left without a number', async () => {
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.getSessionExtensions.mockRejectedValue(new Error('offline') as never);
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    await waitFor(() =>
      expect(screen.getByLabelText(/Manage extensions/)).toHaveAttribute('data-answered', 'true')
    );
    quiet.mockRestore();
  });

  it('has not answered for the next chat in the commit that switches to it', async () => {
    mocks.getSessionExtensions.mockResolvedValueOnce({
      data: {
        extensions: [
          { type: 'stdio', name: 'spoke' },
          { type: 'stdio', name: 'notetaker' },
          { type: 'stdio', name: 'example' },
        ],
      },
    } as never);
    const { rerender } = render(<BottomMenuExtensionSelection sessionId="chat-a" />);
    const chip = screen.getByLabelText(/Manage extensions/);
    await waitFor(() => expect(chip).toHaveAccessibleName('Manage extensions (3 added)'));
    expect(chip).toHaveAttribute('data-answered', 'true');

    let resolveNext: ((value: unknown) => void) | undefined;
    mocks.getSessionExtensions.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveNext = resolve;
      }) as never
    );
    // The same mount binds another chat. Read in the very commit: chat A's list
    // must not count for chat B, not even until an effect clears it.
    rerender(<BottomMenuExtensionSelection sessionId="chat-b" />);
    expect(chip).toHaveAttribute('data-answered', 'false');
    expect(chip).not.toHaveAccessibleName('Manage extensions (3 added)');

    await act(async () => {
      resolveNext?.({ data: { extensions: [{ type: 'stdio', name: 'example' }] } });
      await Promise.resolve();
    });
    expect(chip).toHaveAttribute('data-answered', 'true');
    expect(chip).toHaveAccessibleName('Manage extensions (1 added)');
  });

  it('has answered at once on Home, where the configuration is the count', () => {
    render(<BottomMenuExtensionSelection sessionId={null} />);
    expect(screen.getByLabelText(/Manage extensions/)).toHaveAttribute('data-answered', 'true');
  });

  it('moves the chip the moment a row is toggled, before the refetch', async () => {
    // Two attached, one of them a capability → the chip starts at 1, and
    // enabling `example` must take it to 2 while `addToAgent` is still in
    // flight. A chip that counted capabilities would read 2 → 3 here.
    mocks.getSessionExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'builtin', name: 'autovisualiser' },
          { type: 'stdio', name: 'notetaker' },
        ],
      },
    } as never);
    let resolveEnable: (() => void) | undefined;
    mocks.addToAgent.mockImplementationOnce(
      () => new Promise<void>((resolve) => (resolveEnable = resolve))
    );

    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument()
    );

    toggleList();
    fireEvent.click(await screen.findByRole('option', { name: 'Example' }));

    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (2 added)')).toBeInTheDocument()
    );
    await act(async () => {
      resolveEnable?.();
      await Promise.resolve();
    });
  });

  it('excludes capabilities absent from the global catalog snapshot', async () => {
    mocks.getSessionExtensions.mockResolvedValue({
      data: {
        extensions: [
          { type: 'platform', name: 'todo' },
          { type: 'builtin', name: 'developer' },
          { type: 'platform', name: 'Workspace' },
          { type: 'stdio', name: 'not-in-global-catalog' },
        ],
      },
    } as never);
    render(<BottomMenuExtensionSelection sessionId="catalog-still-loading" />);
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument()
    );
    expect(screen.queryByLabelText('Manage extensions (4 added)')).not.toBeInTheDocument();
  });

  it('refetches and refreshes this chat when the catalog changes', async () => {
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } } as never);

    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();

    const example = await screen.findByRole('option', { name: 'Example' });
    expect(example).toHaveAttribute('aria-selected', 'false');
    await waitFor(() => expect(mocks.getSessionExtensions).toHaveBeenCalledTimes(2));
    const callsBeforeChange = mocks.getSessionExtensions.mock.calls.length;

    mocks.getSessionExtensions.mockResolvedValue({
      data: { extensions: [{ type: 'stdio', name: 'example' }] },
    } as never);
    window.dispatchEvent(new CustomEvent(CATALOG_CHANGED_EVENT, { detail: { revision: 1 } }));

    await waitFor(
      () => expect(mocks.getSessionExtensions.mock.calls.length).toBeGreaterThan(callsBeforeChange),
      { timeout: 1_500 }
    );
    expect(mocks.getSessionExtensions).toHaveBeenLastCalledWith({
      path: { session_id: 'session-1' },
      headers: { 'X-User-Action': 'test-proof' },
    });
    await waitFor(() => expect(example).toHaveAttribute('aria-selected', 'true'));
    expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument();

    const callsBeforeDetach = mocks.getSessionExtensions.mock.calls.length;
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } } as never);
    window.dispatchEvent(new CustomEvent(CATALOG_CHANGED_EVENT, { detail: { revision: 2 } }));

    await waitFor(
      () => expect(mocks.getSessionExtensions.mock.calls.length).toBeGreaterThan(callsBeforeDetach),
      { timeout: 1_500 }
    );
    await waitFor(() => expect(example).toHaveAttribute('aria-selected', 'false'));
    expect(screen.getByLabelText('Manage extensions (0 added)')).toBeInTheDocument();
  });

  /**
   * ⚠ **The popup is not the only consumer, and it is usually CLOSED.** The
   * refetch above is exercised with the dropdown open, which a refactor that
   * moved it inside an `isOpen` guard would still satisfy — while the count on
   * the closed chip, a second window and `useToolCount` in another pane all
   * went stale. That is the exact shape of "did not change with the popup
   * open", so it is asserted without ever opening it.
   */
  it('refetches after a catalog change while the dropdown is closed', async () => {
    mocks.getSessionExtensions.mockResolvedValue({ data: { extensions: [] } } as never);

    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (0 added)')).toBeInTheDocument()
    );
    expect(screen.queryByRole('option')).not.toBeInTheDocument();
    const callsBeforeChange = mocks.getSessionExtensions.mock.calls.length;

    mocks.getSessionExtensions.mockResolvedValue({
      data: { extensions: [{ type: 'stdio', name: 'example' }] },
    } as never);
    window.dispatchEvent(new CustomEvent(CATALOG_CHANGED_EVENT, { detail: { revision: 7 } }));

    await waitFor(
      () => expect(mocks.getSessionExtensions.mock.calls.length).toBeGreaterThan(callsBeforeChange),
      { timeout: 1_500 }
    );
    expect(mocks.getSessionExtensions).toHaveBeenLastCalledWith({
      path: { session_id: 'session-1' },
      headers: { 'X-User-Action': 'test-proof' },
    });
    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument()
    );
  });

  it('does not let a late response from the previous chat replace the current count', async () => {
    let resolvePrevious:
      | ((value: { data: { extensions: Array<{ type: string; name: string }> } }) => void)
      | undefined;
    mocks.getSessionExtensions
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolvePrevious = resolve;
          }) as never
      )
      .mockResolvedValue({
        data: { extensions: [{ type: 'stdio', name: 'example' }] },
      } as never);

    const { rerender } = render(<BottomMenuExtensionSelection sessionId="previous-chat" />);
    rerender(<BottomMenuExtensionSelection sessionId="current-chat" />);

    await waitFor(() =>
      expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument()
    );

    await act(async () => {
      resolvePrevious?.({
        data: {
          extensions: [
            { type: 'stdio', name: 'spoke' },
            { type: 'stdio', name: 'notetaker' },
          ],
        },
      });
      await Promise.resolve();
    });

    expect(screen.getByLabelText('Manage extensions (1 added)')).toBeInTheDocument();
  });

  /**
   * v1.89.0 P-03. Toggling Workspace on here, quitting, and finding it off again
   * reads as a setting that failed to save. It is a per-chat control being read
   * as a global one: `/agent/add_extension` persists into that SESSION's own
   * extension state and never touches `config.yaml`, and the hub view's
   * overrides live in a map `createSession` clears. So the list says what it
   * does: a visible scope word, and the full sentence in its InfoTip (spec 3.7),
   * which a screen reader hears as the glyph's description.
   */
  it('says the session toggle applies to this chat, and where the default lives', async () => {
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();
    await screen.findAllByRole('option');

    expect(screen.getByText(TOOLS_COPY.scope.thisChat)).toBeInTheDocument();
    const help = screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` });
    expect(help).toHaveAccessibleDescription(EXTENSIONS_COPY.helpChat);
    expect(EXTENSIONS_COPY.helpChat).toMatch(/Settings → Extensions/);
    // The explanation is not a visible paragraph any more.
    expect(screen.queryByText(/^Changes apply/, { selector: ':not(.sr-only)' })).toBeNull();
  });

  it('says the hub toggle applies to the next chat, not to "new chats"', async () => {
    render(<BottomMenuExtensionSelection sessionId={null} />);
    toggleList();
    await screen.findAllByRole('option');

    // `clearExtensionOverrides` runs on session creation, so the override shapes
    // exactly one chat — "in new chats" was an over-promise.
    expect(screen.getByText(TOOLS_COPY.scope.nextChat)).toBeInTheDocument();
    const help = screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` });
    expect(help).toHaveAccessibleDescription(EXTENSIONS_COPY.helpNextChat);
    expect(screen.queryByText(TOOLS_COPY.scope.thisChat)).not.toBeInTheDocument();
  });

  /**
   * Decision 6.9: the switch is the confirmation. A success toast for every
   * toggle was noise; an error still says what failed.
   */
  it('raises no success toast for a toggle', async () => {
    const { toastService } = await import('../../toasts');
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();
    fireEvent.click(await screen.findByRole('option', { name: 'Example' }));
    await waitFor(() => expect(mocks.addToAgent).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: /^(Enable|Disable) all/ }));
    await waitFor(() => expect(mocks.getSessionExtensions).toHaveBeenCalled());
    expect(toastService.success).not.toHaveBeenCalled();
  });

  /**
   * Spec 3.7 and primitives defect 6: the description is the row's tooltip and
   * its accessible description, never `title=` (AppTooltipLayer would copy a
   * title into the row's name) and never text inside the option (which would
   * become part of its name).
   */
  it('describes a row without renaming it', async () => {
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();
    const example = await screen.findByRole('option', { name: 'Example' });
    expect(example).toHaveAttribute('aria-description', 'Example extension');
    expect(example.querySelector('[title]')).toBeNull();
    expect(example).not.toHaveAttribute('title');
  });

  it('filters as the person types, with focus staying in the field', async () => {
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();
    const field = await screen.findByPlaceholderText(EXTENSIONS_COPY.search);
    field.focus();
    fireEvent.change(field, { target: { value: 'spo' } });
    expect(screen.getAllByRole('option').map((row) => row.textContent)).toEqual(['Spoke']);
    expect(field).toHaveFocus();
    fireEvent.change(field, { target: { value: 'zzz' } });
    expect(screen.getByText(EXTENSIONS_COPY.noMatch)).toBeInTheDocument();
  });

  /**
   * The hub view is the second, separate branch of the same rule — it reads the
   * config plus the in-memory override map rather than a session — so it gets
   * its own assertion on the number, not only on the rows.
   */
  it('keeps shipped capabilities out of the selector and out of its count', async () => {
    render(<BottomMenuExtensionSelection sessionId={null} />);

    // Four extensions enabled in the config, two of them capabilities → 2.
    const trigger = screen.getByLabelText('Manage extensions (2 added)');
    fireEvent.click(trigger);

    expect(await screen.findAllByRole('option')).toHaveLength(3);
    expect(screen.getByText('Example')).toBeInTheDocument();
    expect(screen.getByText('Spoke')).toBeInTheDocument();
    expect(screen.getByText('Notetaker')).toBeInTheDocument();
    expect(screen.queryByText('autovisualiser')).not.toBeInTheDocument();
    expect(screen.queryByText('code_execution')).not.toBeInTheDocument();
    expect(screen.queryByText('chatrecall')).not.toBeInTheDocument();
    expect(screen.queryByText('agent_drafter')).not.toBeInTheDocument();
  });

  it('serializes rapid session toggles so the latest choice reaches the backend last', async () => {
    let resolveEnable: (() => void) | undefined;
    mocks.addToAgent.mockImplementationOnce(
      () => new Promise<void>((resolve) => (resolveEnable = resolve))
    );
    render(<BottomMenuExtensionSelection sessionId="session-1" />);
    toggleList();

    const toggle = await screen.findByRole('option', { name: 'Example' });
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'false'));

    expect(mocks.addToAgent).toHaveBeenCalledTimes(1);
    expect(mocks.removeFromAgent).not.toHaveBeenCalled();
    resolveEnable?.();

    await waitFor(() => expect(mocks.removeFromAgent).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'false'));
  });

  /**
   * Principle 10: one extension, one name. The composer used to title-case the config key, so an
   * installed CDWAgent read `Cdwagent` here while the Extensions page and the marketplace said
   * CDWAgent (vision V3-R2-6). The fixture lists the two private agents the marketplace names, and
   * one the marketplace has never heard of, so both halves of the resolver are on screen.
   */
  describe('the name a row reads', () => {
    const agent = (name: string, description: string) => ({
      type: 'stdio',
      name,
      description,
      cmd: name,
      args: [],
      enabled: false,
    });

    it('is the catalogue spelling, as on the Extensions page, not the key title-cased', async () => {
      mocks.extraExtensions.push(
        agent('cdwagent', 'Clinical data warehouse'),
        agent('spokeagent', 'Knowledge graph'),
        agent('my-lab_tool', 'Somebody’s own server')
      );
      render(<BottomMenuExtensionSelection sessionId="session-1" />);
      toggleList();

      expect(await screen.findByRole('option', { name: 'CDWAgent' })).toBeInTheDocument();
      expect(screen.getByRole('option', { name: 'SPOKEAgent' })).toBeInTheDocument();
      // Nothing the catalogue does not know is guessed at: the key read as words.
      expect(screen.getByRole('option', { name: 'My Lab Tool' })).toBeInTheDocument();
      expect(screen.queryByText('Cdwagent')).not.toBeInTheDocument();
      expect(screen.queryByText('Spokeagent')).not.toBeInTheDocument();
    });

    it('is in the toggle failure toast, the same name as the row', async () => {
      mocks.extraExtensions.push(agent('cdwagent', 'Clinical data warehouse'));
      mocks.addToAgent.mockRejectedValueOnce(new Error('refused'));
      render(<BottomMenuExtensionSelection sessionId="session-1" />);
      toggleList();

      fireEvent.click(await screen.findByRole('option', { name: 'CDWAgent' }));

      await waitFor(() => expect(toastService.error).toHaveBeenCalledTimes(1));
      expect(toastService.error).toHaveBeenCalledWith({
        title: EXTENSIONS_COPY.toggleErrorTitle,
        msg: EXTENSIONS_COPY.toggleFailed('CDWAgent', true),
      });
    });

    it('is what the list is searched by, beside the key and the description', async () => {
      // `playwrightagent` is `Playwright Agent` in the catalogue: a person who types the words
      // they read finds it, though the key has no space and the description never says them.
      mocks.extraExtensions.push(agent('playwrightagent', 'Browser automation'));
      render(<BottomMenuExtensionSelection sessionId="session-1" />);
      toggleList();
      const field = await screen.findByPlaceholderText(EXTENSIONS_COPY.search);

      fireEvent.change(field, { target: { value: 'playwright agent' } });
      expect(screen.getAllByRole('option').map((row) => row.textContent)).toEqual([
        'Playwright Agent',
      ]);
      // The key still finds it, as it always did.
      fireEvent.change(field, { target: { value: 'playwrightagent' } });
      expect(screen.getAllByRole('option').map((row) => row.textContent)).toEqual([
        'Playwright Agent',
      ]);
    });

    it('is the order the list runs in, as the Extensions page sorts it', async () => {
      // By key `playwrightaardvark` comes first ('a' before 'g'); by the names a person reads,
      // `Playwright Agent` does (a space sorts before a letter).
      mocks.extraExtensions.push(
        agent('playwrightaardvark', 'Not in the catalogue'),
        agent('playwrightagent', 'Browser automation')
      );
      render(<BottomMenuExtensionSelection sessionId="session-1" />);
      toggleList();
      await screen.findAllByRole('option');

      const order = screen.getAllByRole('option').map((row) => row.textContent);
      expect(order.indexOf('Playwright Agent')).toBeGreaterThanOrEqual(0);
      expect(order.indexOf('Playwright Agent')).toBeLessThan(order.indexOf('Playwrightaardvark'));
    });
  });
});
