import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { INSTITUTION_ID_PATTERN } from '../identity';
import type { ConnectionStatusKey } from '../state/crewStatus';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { sidebarCopy } from './copy';
import { CHIP_DEFERRED_STATUSES, CHIP_SILENT_STATUSES, PrivacyChip } from './PrivacyChip';
import { PRIVACY_UPDATE_KEY } from './PrivacyPopover';
import { verifiedPrivacy } from './sidebarView';
import {
  bob,
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  type ControllerOverrides,
} from './sidebarTestUtils';

// The chip reads the configured providers for the names they publish for an institution ID
// (Q2-38). Stable callbacks, as the real context's are.
const config = vi.hoisted(() => {
  const state = { providers: [] as unknown[] };
  return {
    state,
    getProviders: async () => state.providers,
    read: async () => null,
  };
});
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return {
    ...actual,
    useConfig: () => ({ getProviders: config.getProviders, read: config.read }),
  };
});

afterEach(() => {
  config.state.providers = [];
});

/** A configured provider whose affiliation publishes "UCSF" as the name of `ucsf`. */
const ucsfProvider = {
  name: 'versa_azure',
  is_configured: true,
  affiliation: { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] },
};

const copy = sidebarCopy.privacy;

/** A verified controller with the given connection and workspace modes. */
function privacyController(
  connectionMode: 'private' | 'public',
  workspaceMode: 'private' | 'public',
  extra: ControllerOverrides = {}
) {
  const snapshot = makeSnapshot({
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: workspaceMode,
      institution_id: 'ucsf',
      policy_epoch: 1,
    },
  });
  const merged = { ...connection, mode: connectionMode };
  return makeController({
    snapshot,
    connection: merged,
    connections: [merged],
    observedPrivacy: {
      connectionId: connection.id,
      mode: connectionMode,
      institutionId: connectionMode === 'private' ? 'ucsf' : null,
      policyEpoch: 1,
    },
    effectivePrivacy:
      connectionMode === 'private' || workspaceMode === 'private' ? 'private' : 'public',
    ...extra,
  });
}

async function openPopover(name: RegExp) {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name }));
  return screen.findByText((_, node) => node?.hasAttribute('data-crew-privacy-popover') ?? false);
}

describe('PrivacyChip', () => {
  it('is named "Privacy: Private · ucsf" and shows the padlock badge with the institution', () => {
    renderWithCrew(<PrivacyChip />);
    const chip = screen.getByRole('button', { name: 'Privacy: Private · ucsf' });
    const badge = within(chip).getByTestId('privacy-badge');
    expect(badge).toHaveAttribute('data-privacy', 'private');
    // enforcementOff={false}: the broker enforces Crew mode whatever this machine's switch says.
    expect(badge).toHaveAttribute('data-enforcement', 'on');
    expect(chip).toHaveTextContent('Private · ucsf');
    expect(chip).toHaveClass('no-drag');
  });

  it('is ONE badge, the institution inside it, with no press scale (Q2-45)', () => {
    renderWithCrew(<PrivacyChip />);
    const chip = screen.getByRole('button', { name: 'Privacy: Private · ucsf' });
    const pill = chip.querySelector('.crew-sidebar-chip-badge') as HTMLElement;
    expect(pill).not.toBeNull();
    // The padlock badge and the institution share one fill: both inside the one pill.
    expect(pill).toContainElement(within(chip).getByTestId('privacy-badge'));
    expect(pill).toContainElement(within(chip).getByText('ucsf'));
    expect(Array.from(chip.children)).toEqual([pill]);
    // No tooltip that repeats the badge's own words.
    expect(chip.querySelector('[title]')).toBeNull();
    // Security state never animates: the shared press scale is overridden, not merely hidden.
    expect(chip).toHaveClass('active:scale-100');
    expect(chip.className).not.toContain('active:scale-[0.98]');
  });

  it('words the institution by the name a configured provider publishes for it (Q2-38)', async () => {
    config.state.providers = [ucsfProvider];
    renderWithCrew(<PrivacyChip />);
    const chip = await screen.findByRole('button', { name: 'Privacy: Private · UCSF' });
    expect(chip).toHaveTextContent('UCSF');
    expect(chip).not.toHaveTextContent('ucsf');
    const popover = await openPopover(/^Privacy: Private · UCSF/);
    expect(screen.getByRole('dialog')).toHaveAccessibleName('Privacy: Private · UCSF');
    expect(
      within(popover).getByText('Only private and UCSF-approved models can read Fixture.')
    ).toBeInTheDocument();
    const values = Array.from(popover.querySelectorAll('dd')).map((dd) => dd.textContent);
    expect(values[2]).toBe('UCSF');
  });

  it('keeps an institution ID nobody publishes a name for exactly as stored', () => {
    config.state.providers = [
      {
        ...ucsfProvider,
        affiliation: { kind: 'institutions', institutions: [{ id: 'sdsc', display_name: 'SDSC' }] },
      },
    ];
    renderWithCrew(<PrivacyChip />);
    expect(screen.getByRole('button', { name: 'Privacy: Private · ucsf' })).toBeInTheDocument();
  });

  it('is named "Privacy: Public" with no institution when the effective mode is Public', () => {
    renderWithCrew(<PrivacyChip />, privacyController('public', 'public'));
    const chip = screen.getByRole('button', { name: 'Privacy: Public' });
    expect(within(chip).getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'public');
    expect(chip).not.toHaveTextContent('ucsf');
  });

  it('is named "Privacy: Private" when no institution is set anywhere', () => {
    const snapshot = makeSnapshot({
      workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', policy_epoch: 1 },
    });
    renderWithCrew(
      <PrivacyChip />,
      makeController({
        snapshot,
        observedPrivacy: {
          connectionId: connection.id,
          mode: 'private',
          institutionId: null,
          policyEpoch: 1,
        },
      })
    );
    expect(screen.getByRole('button', { name: 'Privacy: Private' })).toBeInTheDocument();
  });

  it.each([
    ['no observed privacy yet', { observedPrivacy: null }],
    [
      'privacy observed for a different connection',
      {
        observedPrivacy: {
          connectionId: 'conn-other',
          mode: 'private' as const,
          institutionId: 'ucsf',
          policyEpoch: 1,
        },
      },
    ],
    ['no verified snapshot', { snapshot: null }],
    ['no effective privacy', { effectivePrivacy: null }],
  ])('reads "Checking privacy…" with no padlock and nothing to open: %s', async (_, overrides) => {
    const user = userEvent.setup();
    // Without a snapshot the controller would read "Checking connection", where the status word
    // stands in for the chip (Q3-55); "Updating…" is where the chip still says it.
    renderWithCrew(
      <PrivacyChip />,
      makeController({ ...overrides, ...('snapshot' in overrides ? { status: 'updating' } : {}) })
    );
    expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    // What it waits for, on hover, BELOW the row: never a native title, which the app's tooltip
    // layer opened up over the workspace name (T-06, T-68, Q2-17).
    const chip = document.querySelector('[data-crew-privacy="checking"]') as HTMLElement;
    expect(chip).not.toHaveAttribute('title');
    await user.hover(chip);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(sidebarCopy.chip.checkingHint);
    expect(document.querySelector('[data-crew-privacy-tooltip]')).toHaveAttribute(
      'data-side',
      'bottom'
    );
    expect(sidebarCopy.chip.checkingHint.startsWith(sidebarCopy.chip.checking.slice(0, -1))).toBe(
      true
    );
  });

  it('keeps "Checking privacy…" while the status is updating: the next snapshot verifies it', () => {
    renderWithCrew(
      <PrivacyChip />,
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        status: 'updating',
      })
    );
    expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
  });

  // Q3-55: "Checking connection · Checking pri…" cut both facts short at 240px. The status word
  // already says a check runs, so the unverified chip steps aside and the row reads it whole.
  it.each(['connecting', 'checking'] as const)(
    'renders nothing while the status is %s and privacy is unverified: the word says it',
    (status) => {
      const { container } = renderWithCrew(
        <PrivacyChip />,
        makeController({ snapshot: null, observedPrivacy: null, effectivePrivacy: null, status })
      );
      expect(container).toBeEmptyDOMElement();
      expect(CHIP_DEFERRED_STATUSES.has(status)).toBe(true);
    }
  );

  it('keeps a VERIFIED chip during a connect: it is verified, and its resting home', () => {
    renderWithCrew(<PrivacyChip />, makeController({ status: 'connecting' }));
    expect(screen.getByRole('button', { name: 'Privacy: Private · ucsf' })).toBeInTheDocument();
  });

  it('defers to the status word in exactly these statuses, decided on purpose', () => {
    expect([...CHIP_DEFERRED_STATUSES].sort()).toEqual(['checking', 'connecting']);
    for (const status of CHIP_DEFERRED_STATUSES)
      expect(CHIP_SILENT_STATUSES.has(status)).toBe(false);
  });

  // Q2-17, Q2-01, Q2-43: "Offline · Checking privacy…" claimed work nothing was doing, and a
  // joiner read "Privacy shown after…" cut off beside "Not joined yet".
  it.each([
    'offline',
    'reconnecting',
    'sign-in-needed',
    'cant-connect',
    'cant-verify',
    'not-set-up',
    'not-joined',
    'updates-unavailable',
  ] as const)(
    'says nothing at all while the status is %s: nothing is checking privacy',
    (status) => {
      const { container } = renderWithCrew(
        <PrivacyChip />,
        makeController({ snapshot: null, observedPrivacy: null, effectivePrivacy: null, status })
      );
      expect(container).toBeEmptyDOMElement();
      expect(CHIP_SILENT_STATUSES.has(status)).toBe(true);
    }
  );

  it('checks every silent status against the status table, so a new one is decided on purpose', () => {
    const decided: ConnectionStatusKey[] = [
      'offline',
      'reconnecting',
      'sign-in-needed',
      'cant-connect',
      'cant-verify',
      'not-set-up',
      'not-joined',
      'updates-unavailable',
    ];
    expect([...CHIP_SILENT_STATUSES].sort()).toEqual([...decided].sort());
  });

  it('never shows an unverified mode as verified, whatever the status claims', () => {
    // Even a controller reading "connected" shows no mode until privacy is observed.
    renderWithCrew(
      <PrivacyChip />,
      makeController({ observedPrivacy: null, effectivePrivacy: 'private', status: 'connected' })
    );
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('never takes the mode from the last verified copy', () => {
    const snapshot = makeSnapshot();
    renderWithCrew(
      <PrivacyChip />,
      makeController({
        // Re-verifying a view it had: the one unverified state where the chip speaks.
        status: 'updating',
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        lastVerified: {
          connectionId: connection.id,
          snapshot,
          observedPrivacy: {
            connectionId: connection.id,
            mode: 'private',
            institutionId: 'ucsf',
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: '',
          channelId: '',
          messages: [],
        },
      })
    );
    expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });
});

describe('the privacy popover', () => {
  it('is named by its title, opens on itself rather than on an action, and aligns to the chip’s start (T-38)', async () => {
    renderWithCrew(<PrivacyChip />);
    const popover = await openPopover(/^Privacy: Private/);
    const dialog = popover.closest('[role="dialog"]') as HTMLElement;
    expect(dialog).toHaveAccessibleName('Privacy: Private · ucsf');
    const title = popover.querySelector('[data-crew-privacy-title]') as HTMLElement;
    expect(dialog).toHaveAttribute('aria-labelledby', title.id);
    // Initial focus is the popover itself — never "Make my connection public…".
    await waitFor(() => expect(dialog).toHaveFocus());
    expect(within(popover).getByRole('button', { name: copy.makePublic })).not.toHaveFocus();
    expect(dialog).toHaveAttribute('tabindex', '-1');
    // Start-aligned, so it stays over the Crew column instead of the app sidebar.
    expect(dialog).toHaveAttribute('data-align', 'start');
  });

  it('keeps its name through the institution step', async () => {
    const publicConnection = { ...connection, mode: 'public' as const, institution_id: null };
    renderWithCrew(
      <PrivacyChip />,
      privacyController('public', 'public', { connection: publicConnection })
    );
    const popover = await openPopover(/^Privacy: Public/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.makePrivate }));
    await screen.findByPlaceholderText(copy.institutionPlaceholder);
    expect(screen.getByRole('dialog')).toHaveAccessibleName('Privacy: Public');
  });

  it.each([
    [
      'public',
      'Makes only your connection Public: public models could then read the public-safe channels you can see in Fixture. Restricted channels stay private.',
    ],
    [
      'private',
      'Makes only your connection Public. Fixture is Private for everyone, so the models that can read it stay the same.',
    ],
  ] as const)(
    'names what "Make my connection public…" changes when the workspace is %s',
    async (workspaceMode, line) => {
      renderWithCrew(<PrivacyChip />, privacyController('private', workspaceMode));
      const popover = await openPopover(/^Privacy: Private/);
      const button = within(popover).getByRole('button', { name: 'Make my connection public…' });
      expect(within(popover).getByText(line)).toHaveAttribute('data-crew-privacy-effect');
      expect(button).toHaveAccessibleDescription(line);
      // Checked against the broker: a Public connection loses no channel, only which models may
      // read what. The effect never says otherwise.
      expect(line).not.toMatch(/lose|access/i);
    }
  );

  it('makes the downgrade a quiet link, never the popover’s most prominent control (Q2-44)', async () => {
    renderWithCrew(<PrivacyChip />);
    const popover = await openPopover(/^Privacy: Private/);
    const downgrade = within(popover).getByRole('button', { name: copy.makePublic });
    const more = within(popover).getByRole('button', { name: copy.more });
    for (const link of [downgrade, more]) {
      // The link variant: no box, no fill, no outline.
      expect(link.className).toContain('underline-offset-4');
      expect(link.className).not.toMatch(/border-border-emphasized|bg-background-medium/);
    }
    expect(downgrade).toHaveClass('text-text-muted');
  });

  it.each([
    ['private', 'private', 'both', copy.why.both('Fixture')],
    ['private', 'public', 'connection', copy.why.connection()],
    ['public', 'private', 'workspace', copy.why.workspace('Fixture')],
    ['public', 'public', 'public', copy.why.public('Fixture')],
  ] as const)(
    'connection %s, workspace %s: always opens the note with the "%s" why',
    async (connectionMode, workspaceMode, why, line) => {
      renderWithCrew(<PrivacyChip />, privacyController(connectionMode, workspaceMode));
      const popover = await openPopover(/^Privacy:/);
      const note = popover.querySelector('[data-crew-privacy-why]') as HTMLElement;
      expect(note).toHaveAttribute('data-crew-privacy-why', why);
      expect(note.textContent?.startsWith(line)).toBe(true);
    }
  );

  it('says it all in one note: the why, then who can see the workspace (Q2-44)', async () => {
    renderWithCrew(<PrivacyChip />);
    const popover = await openPopover(/^Privacy: Private/);
    const notes = popover.querySelectorAll('[data-crew-privacy-why]');
    expect(notes).toHaveLength(1);
    // The host changes the workspace setting themselves, so nothing says only the host can.
    expect(notes[0]).toHaveTextContent(
      `${copy.why.both('Fixture')} ${copy.audience('Fixture', null)}`
    );
    // Summary, facts, note, effect: never the five blocks it was.
    expect(popover.querySelectorAll('p')).toHaveLength(3);
  });

  it('states the three facts, with the institution as its bare ID and no stray period', async () => {
    renderWithCrew(<PrivacyChip />);
    const popover = await openPopover(/^Privacy: Private/);
    expect(
      within(popover).getByText('Only private and ucsf-approved models can read Fixture.')
    ).toBeInTheDocument();
    const facts = popover.querySelector('dl');
    expect(facts).not.toBeNull();
    const values = Array.from(facts?.querySelectorAll('dd') ?? []).map((dd) => dd.textContent);
    expect(values).toEqual([
      copy.values.private,
      copy.values.workspacePrivate,
      // The institution renders as itself: no trailing "." and no casing guesswork.
      'ucsf',
    ]);
  });

  it('says "Not set" when neither the workspace nor the connection has an institution', async () => {
    const snapshot = makeSnapshot({
      workspace: { id: 'workspace-1', host_uid: 1000, mode: 'public', policy_epoch: 1 },
    });
    const publicConnection = { ...connection, mode: 'public' as const, institution_id: null };
    renderWithCrew(
      <PrivacyChip />,
      makeController({
        snapshot,
        connection: publicConnection,
        observedPrivacy: {
          connectionId: connection.id,
          mode: 'public',
          institutionId: null,
          policyEpoch: 1,
        },
        effectivePrivacy: 'public',
      })
    );
    const popover = await openPopover(/^Privacy: Public/);
    expect(within(popover).getByText(copy.values.notSet)).toBeInTheDocument();
    expect(
      within(popover).getByText(
        'Public models can read public-safe channels in Fixture. Restricted channels stay private.'
      )
    ).toBeInTheDocument();
  });

  it('Make public… only opens the typed confirmation; it sends nothing itself', async () => {
    const controller = privacyController('private', 'public');
    renderWithCrew(<PrivacyChip />, controller);
    const popover = await openPopover(/^Privacy: Private/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.makePublic }));
    expect(controller.openDialog).toHaveBeenCalledWith({
      kind: 'confirm',
      confirm: { action: 'make-connection-public', connectionId: connection.id },
    });
    expect(controller.updateConnection).not.toHaveBeenCalled();
    expect(controller.act).not.toHaveBeenCalled();
  });

  it('Make private is one click: the full connection body, then a refresh', async () => {
    const publicConnection = { ...connection, mode: 'public' as const };
    const controller = privacyController('public', 'public', {
      connection: publicConnection,
    });
    renderWithCrew(<PrivacyChip />, controller);
    const popover = await openPopover(/^Privacy: Public/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.makePrivate }));
    await waitFor(() => expect(controller.refresh).toHaveBeenCalledTimes(1));
    expect(controller.act).toHaveBeenCalledWith('global', PRIVACY_UPDATE_KEY, expect.any(Function));
    expect(controller.updateConnection).toHaveBeenCalledWith(connection.id, {
      ...connectionUpdateBody(publicConnection),
      mode: 'private',
    });
    expect(controller.openDialog).not.toHaveBeenCalled();
  });

  it('asks for an institution first when the connection has none, and sends it', async () => {
    const publicConnection = { ...connection, mode: 'public' as const, institution_id: null };
    const controller = privacyController('public', 'public', {
      connection: publicConnection,
    });
    renderWithCrew(<PrivacyChip />, controller);
    const popover = await openPopover(/^Privacy: Public/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.makePrivate }));

    const field = await screen.findByPlaceholderText(copy.institutionPlaceholder);
    expect(field).toBeRequired();
    // The rendered attribute is the one shared rule, and it compiles the way a browser compiles a
    // `pattern` (the `v` flag). A pattern that fails to compile there is dropped silently, so the
    // field would accept anything while this test still read the attribute back.
    expect(field).toHaveAttribute('pattern', INSTITUTION_ID_PATTERN);
    expect(() => new RegExp(`^(?:${field.getAttribute('pattern')})$`, 'v')).not.toThrow();
    expect(controller.updateConnection).not.toHaveBeenCalled();

    fireEvent.change(field, { target: { value: 'UCSF' } });
    expect((field as HTMLInputElement).validity.patternMismatch).toBe(true);

    fireEvent.change(field, { target: { value: 'sdsc' } });
    expect((field as HTMLInputElement).validity.patternMismatch).toBe(false);
    fireEvent.submit(field.closest('form') as HTMLFormElement);
    await waitFor(() => expect(controller.refresh).toHaveBeenCalledTimes(1));
    expect(controller.updateConnection).toHaveBeenCalledWith(connection.id, {
      ...connectionUpdateBody(publicConnection),
      mode: 'private',
      institution_id: 'sdsc',
    });
  });

  it('Cancel on the institution step returns to the popover and sends nothing', async () => {
    const publicConnection = { ...connection, mode: 'public' as const, institution_id: null };
    const controller = privacyController('public', 'public', { connection: publicConnection });
    renderWithCrew(<PrivacyChip />, controller);
    const popover = await openPopover(/^Privacy: Public/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.makePrivate }));
    fireEvent.click(await screen.findByRole('button', { name: copy.cancel }));
    await waitFor(() =>
      expect(document.querySelector('[data-crew-privacy-why]')).toHaveTextContent(
        copy.why.public('Fixture')
      )
    );
    expect(controller.updateConnection).not.toHaveBeenCalled();
  });

  it('Privacy… opens Workspace settings on the Privacy tab', async () => {
    const controller = makeController();
    renderWithCrew(<PrivacyChip />, controller);
    const popover = await openPopover(/^Privacy:/);
    fireEvent.click(within(popover).getByRole('button', { name: copy.more }));
    expect(controller.openDialog).toHaveBeenCalledWith({
      kind: 'workspace-settings',
      tab: 'privacy',
    });
  });

  it('tells a member, not the host, that only the host changes the workspace setting', async () => {
    const member = privacyController('public', 'private', {
      isHost: false,
      snapshot: makeSnapshot({
        actor: bob,
        workspace: {
          id: 'workspace-1',
          host_uid: 1000,
          mode: 'private',
          institution_id: 'ucsf',
          policy_epoch: 1,
        },
      }),
    });
    const view = renderWithCrew(<PrivacyChip />, member);
    let popover = await openPopover(/^Privacy: Private/);
    let note = popover.querySelector('[data-crew-privacy-why]') as HTMLElement;
    expect(note).toHaveTextContent(copy.hostOnly('Fixture'));
    // …and who can see it at all: "Private" is about models, never about people (Q2-44).
    expect(note.textContent).toMatch(/Only people .*@alice.* lets in can see Fixture\.$/);
    view.unmount();

    renderWithCrew(<PrivacyChip />, privacyController('public', 'private', { isHost: true }));
    popover = await openPopover(/^Privacy: Private/);
    note = popover.querySelector('[data-crew-privacy-why]') as HTMLElement;
    expect(note).not.toHaveTextContent(copy.hostOnly('Fixture'));
    expect(note).toHaveTextContent(copy.audience('Fixture', null));
  });

  it('disables the change while one is already running', async () => {
    renderWithCrew(
      <PrivacyChip />,
      makeController({ isPending: (key) => key === PRIVACY_UPDATE_KEY })
    );
    const popover = await openPopover(/^Privacy:/);
    expect(within(popover).getByRole('button', { name: copy.makePublic })).toBeDisabled();
  });
});

describe('verifiedPrivacy', () => {
  it('prefers the workspace institution and falls back to the connection’s', () => {
    const withWorkspace = verifiedPrivacy(makeController());
    expect(withWorkspace?.institution).toBe('ucsf');

    const snapshot = makeSnapshot({
      workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', policy_epoch: 1 },
    });
    const fallback = verifiedPrivacy(
      makeController({
        snapshot,
        observedPrivacy: {
          connectionId: connection.id,
          mode: 'private',
          institutionId: 'sdsc',
          policyEpoch: 1,
        },
      })
    );
    expect(fallback?.institution).toBe('sdsc');
  });
});
