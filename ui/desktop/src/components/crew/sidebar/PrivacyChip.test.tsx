import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { INSTITUTION_ID_PATTERN } from '../identity';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { sidebarCopy } from './copy';
import { PrivacyChip } from './PrivacyChip';
import { PRIVACY_UPDATE_KEY } from './PrivacyPopover';
import { verifiedPrivacy } from './sidebarView';
import {
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  type ControllerOverrides,
} from './sidebarTestUtils';

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
  ])('reads "Checking privacy…" with no padlock and nothing to open: %s', (_, overrides) => {
    renderWithCrew(<PrivacyChip />, makeController(overrides));
    expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    // Its full words, and what it waits for, on hover: the row can truncate it (T-06, T-68).
    const chip = document.querySelector('[data-crew-privacy="checking"]');
    expect(chip).toHaveAttribute('title', sidebarCopy.chip.checkingHint);
    expect(sidebarCopy.chip.checkingHint.startsWith(sidebarCopy.chip.checking.slice(0, -1))).toBe(
      true
    );
  });

  it('tells a joiner the host has not let in that privacy shows after they join (T-06)', () => {
    // A non-member's privacy is never verified, so "Checking privacy…" would never resolve.
    renderWithCrew(
      <PrivacyChip />,
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        status: 'not-joined',
      })
    );
    expect(screen.getByText('Privacy shown after you join')).toBeInTheDocument();
    expect(screen.queryByText(sidebarCopy.chip.checking)).toBeNull();
    // Still plain text: no padlock and nothing to open, so it never looks verified.
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(document.querySelector('[data-crew-privacy="not-joined"]')).toHaveAttribute(
      'title',
      sidebarCopy.chip.notJoinedHint
    );
  });

  it.each(['connecting', 'checking', 'updates-unavailable'] as const)(
    'keeps "Checking privacy…" for a member while the status is %s',
    (status) => {
      renderWithCrew(
        <PrivacyChip />,
        makeController({ snapshot: null, observedPrivacy: null, effectivePrivacy: null, status })
      );
      expect(screen.getByText(sidebarCopy.chip.checking)).toBeInTheDocument();
    }
  );

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
      'Changes only your connection: public models could then read public-safe channels in Fixture. You’ll confirm first.',
    ],
    [
      'private',
      'Changes only your connection. Fixture stays Private for everyone, so the models that can read it stay the same.',
    ],
  ] as const)(
    'names what "Make my connection public…" changes when the workspace is %s',
    async (workspaceMode, line) => {
      renderWithCrew(<PrivacyChip />, privacyController('private', workspaceMode));
      const popover = await openPopover(/^Privacy: Private/);
      const button = within(popover).getByRole('button', { name: 'Make my connection public…' });
      expect(within(popover).getByText(line)).toHaveAttribute('data-crew-privacy-effect');
      expect(button).toHaveAccessibleDescription(line);
    }
  );

  it.each([
    ['private', 'private', 'both', copy.why.both],
    ['private', 'public', 'connection', copy.why.connection],
    ['public', 'private', 'workspace', copy.why.workspace],
    ['public', 'public', 'public', copy.why.public],
  ] as const)(
    'connection %s, workspace %s: always shows the "%s" why line',
    async (connectionMode, workspaceMode, why, line) => {
      renderWithCrew(<PrivacyChip />, privacyController(connectionMode, workspaceMode));
      const popover = await openPopover(/^Privacy:/);
      expect(within(popover).getByText(line)).toHaveAttribute('data-crew-privacy-why', why);
    }
  );

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
    expect(await screen.findByText(copy.why.public)).toBeInTheDocument();
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
    const member = privacyController('public', 'private', { isHost: false });
    const view = renderWithCrew(<PrivacyChip />, member);
    let popover = await openPopover(/^Privacy: Private/);
    expect(within(popover).getByText(copy.hostOnly)).toBeInTheDocument();
    view.unmount();

    renderWithCrew(<PrivacyChip />, privacyController('public', 'private', { isHost: true }));
    popover = await openPopover(/^Privacy: Private/);
    expect(within(popover).queryByText(copy.hostOnly)).toBeNull();
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
