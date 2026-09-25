import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Channel } from '../crewApi';
import { buildPeopleDirectory } from '../identity';
import type { ConnectFailureKind } from '../state/connectFailure';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { ChannelIntro } from '../timeline/ChannelIntro';
import { checklistCopy, emptyCopy, INSTALL_COMMANDS, notSetUpCopy, welcomeCopy } from './copy';
import {
  ConnectingCard,
  focusComposerOnceMounted,
  NoChannelState,
  NoTeamState,
  OfflineState,
  resetConnectAttemptsForTests,
  SignInNeededState,
} from './EmptyStates';
import { attemptTime } from './joinText';
import { resetJoinContextForTests, updateJoinContext } from './joinContext';
import { NotSetUpPane } from './NotSetUpPane';
import { OnboardingScreen, ONBOARDING_SCREENS } from './OnboardingScreen';
import { SetupChecklist } from './SetupChecklist';
import {
  fakeConnection,
  fakeSnapshot,
  makeCrew,
  renderWithCrew,
  type CrewRender,
} from './testCrew';
import { Welcome } from './Welcome';

vi.mock('../../InAppTerminalDock', () => ({ default: () => <div /> }));

const connection = fakeConnection();

function crewWith(overrides: Partial<CrewController> = {}) {
  return makeCrew({ connectionId: 'conn-1', connection, connections: [connection], ...overrides });
}

beforeEach(() => {
  resetJoinContextForTests();
  resetConnectAttemptsForTests();
  localStorage.clear();
});

describe('Welcome', () => {
  it('leads with Join a workspace and keeps hosting as a link', () => {
    const crew = makeCrew();
    renderWithCrew(<Welcome />, crew);
    expect(screen.getByRole('heading', { name: welcomeCopy.title })).toBeInTheDocument();
    expect(screen.getByText(welcomeCopy.body)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: welcomeCopy.join }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'join' });
    fireEvent.click(screen.getByRole('button', { name: welcomeCopy.host }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'host' });
    // Privacy is stated where it is chosen, not on the first screen.
    expect(document.body.textContent).not.toMatch(/Private by default/);
  });
});

describe('connection states', () => {
  it('says which server a connect is reaching', () => {
    renderWithCrew(<ConnectingCard />, crewWith());
    expect(screen.getByText(emptyCopy.connecting('hpc.ucsf.edu'))).toBeInTheDocument();
  });

  it('offers Connect when the workspace is offline', () => {
    const crew = crewWith({ connection: fakeConnection({ status: 'disconnected' }) });
    renderWithCrew(<OfflineState />, crew);
    expect(
      screen.getByRole('heading', { name: emptyCopy.offlineTitle('lab') })
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.offlineAction('lab') }));
    expect(crew.connect).toHaveBeenCalledWith({ userInitiated: true });
  });

  it('names the server by the person’s own SSH alias when the daemon found one (D-ALIAS)', () => {
    const aliased = {
      ...fakeConnection({ ssh_target: 'bob@52.33.141.141' }),
      server_label: 'lab-server',
    };
    renderWithCrew(<ConnectingCard />, crewWith({ connection: aliased, connections: [aliased] }));
    expect(screen.getByText(emptyCopy.connecting('lab-server'))).toBeInTheDocument();
  });

  describe('Connect keeps keyboard focus off the page (Q2-20)', () => {
    afterEach(() => {
      document.querySelectorAll('[data-test-landing]').forEach((node) => node.remove());
    });

    it('takes focus when the screen arrives with focus nowhere', () => {
      (document.activeElement as HTMLElement | null)?.blur();
      const crew = crewWith({ connection: fakeConnection({ status: 'disconnected' }) });
      renderWithCrew(<OfflineState />, crew);
      expect(screen.getByRole('button', { name: emptyCopy.offlineAction('lab') })).toHaveFocus();
    });

    it('leaves focus where it already is', () => {
      const elsewhere = document.createElement('button');
      elsewhere.setAttribute('data-test-landing', '');
      document.body.appendChild(elsewhere);
      elsewhere.focus();
      renderWithCrew(
        <OfflineState />,
        crewWith({ connection: fakeConnection({ status: 'disconnected' }) })
      );
      expect(elsewhere).toHaveFocus();
    });

    it('stays focusable while connecting, and connects once however often it is pressed', () => {
      const crew = crewWith({
        connection: fakeConnection({ status: 'disconnected' }),
        isPending: vi.fn((key: string) => key === 'connect'),
      });
      renderWithCrew(<OfflineState />, crew);
      const connect = screen.getByRole('button', { name: emptyCopy.offlineAction('lab') });
      connect.focus();
      // Busy, not disabled: a disabled control would drop focus to the page.
      expect(connect).toHaveAttribute('aria-disabled', 'true');
      expect(connect).not.toBeDisabled();
      fireEvent.click(connect);
      expect(crew.connect).not.toHaveBeenCalled();
      expect(connect).toHaveFocus();
    });

    it('lands on the channel once Connect has left with the offline screen', async () => {
      let finish!: () => void;
      const connect = vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          })
      );
      const view = renderWithCrew(
        <OfflineState />,
        crewWith({ connection: fakeConnection({ status: 'disconnected' }), connect })
      );
      const button = screen.getByRole('button', { name: emptyCopy.offlineAction('lab') });
      button.focus();
      fireEvent.click(button);
      // Connecting replaces the offline screen, and the channel opens a moment later.
      view.unmount();
      expect(document.activeElement).toBe(document.body);
      await act(async () => finish());
      const app = document.createElement('div');
      app.className = 'crew-app';
      app.setAttribute('data-test-landing', '');
      const heading = document.createElement('h1');
      const headingButton = document.createElement('button');
      headingButton.textContent = '# general';
      heading.appendChild(headingButton);
      app.appendChild(heading);
      await act(async () => {
        document.body.appendChild(app);
      });
      await waitFor(() => expect(headingButton).toHaveFocus());
    });

    it('keeps focus on Connect when the connect failed and the screen stayed', async () => {
      const crew = crewWith({ connection: fakeConnection({ status: 'disconnected' }) });
      renderWithCrew(<OfflineState />, crew);
      const button = screen.getByRole('button', { name: emptyCopy.offlineAction('lab') });
      button.focus();
      fireEvent.click(button);
      await waitFor(() => expect(crew.connect).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(button).toHaveFocus();
    });
  });

  describe('Connect never leaves focus on the page while it connects (Q4-09)', () => {
    /** The main area as the layout draws it: the onboarding screen, or the channel. */
    function MainArea() {
      const { screen: current } = useCrew();
      if (current === 'channel') {
        return (
          <div className="crew-app">
            <h1>
              <button type="button"># general</button>
            </h1>
          </div>
        );
      }
      return <OnboardingScreen />;
    }

    it('holds focus on the connecting card, then lands on the channel', async () => {
      let finish!: () => void;
      const connect = vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          })
      );
      const view = renderWithCrew(
        <MainArea />,
        crewWith({
          connection: fakeConnection({ status: 'disconnected' }),
          screen: 'offline',
          connect,
        })
      );
      const button = screen.getByRole('button', { name: emptyCopy.offlineAction('lab') });
      button.focus();
      fireEvent.click(button);
      // Connect leaves with the offline screen; the connecting card replaces it in one commit.
      view.update({ screen: 'connecting' });
      const title = screen.getByText(emptyCopy.connecting('hpc.ucsf.edu'));
      expect(document.activeElement).not.toBe(document.body);
      expect(title).toHaveFocus();
      // A place to stand, not a Tab stop, and inside the card's heading.
      expect(title).toHaveAttribute('tabindex', '-1');
      expect(title.closest('h2')).not.toBeNull();

      await act(async () => finish());
      view.update({ screen: 'channel' });
      await waitFor(() => expect(screen.getByRole('button', { name: '# general' })).toHaveFocus());
    });

    it('lands on the channel even when the connect settles after the channel opened', async () => {
      let finish!: () => void;
      const connect = vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          })
      );
      const view = renderWithCrew(
        <MainArea />,
        crewWith({
          connection: fakeConnection({ status: 'disconnected' }),
          screen: 'offline',
          connect,
        })
      );
      const button = screen.getByRole('button', { name: emptyCopy.offlineAction('lab') });
      button.focus();
      fireEvent.click(button);
      view.update({ screen: 'connecting' });
      view.update({ screen: 'channel' });
      await act(async () => finish());
      await waitFor(() => expect(screen.getByRole('button', { name: '# general' })).toHaveFocus());
    });

    it('takes no focus from a control that still has it', () => {
      const elsewhere = document.createElement('button');
      elsewhere.setAttribute('data-test-landing', '');
      document.body.appendChild(elsewhere);
      elsewhere.focus();
      renderWithCrew(<ConnectingCard />, crewWith());
      expect(elsewhere).toHaveFocus();
      elsewhere.remove();
    });

    it('draws no ring on the title that holds focus (authored CSS)', () => {
      const css = readFileSync(join(__dirname, 'onboarding.css'), 'utf8');
      const rule = css.match(/\.crew-onboard-focus-hold:focus[^{]*\{([^}]*)\}/);
      expect(rule?.[1]).toMatch(/outline:\s*none/);
    });
  });

  describe('the offline card says when Connect last tried (Q4-07, Q4-06)', () => {
    const offline = fakeConnection({ status: 'disconnected' });

    /** One Connect that fails: offline → connecting → offline with the failure recorded. */
    function attempt(view: CrewRender, at: number, kind: ConnectFailureKind) {
      const now = vi.spyOn(Date, 'now').mockReturnValue(at);
      view.update({ screen: 'connecting' });
      view.update({
        screen: 'offline',
        lastConnectFailure: { kind, message: 'Crew SSH failure' },
      });
      now.mockRestore();
    }

    it('says nothing before any attempt failed', () => {
      renderWithCrew(<OnboardingScreen />, crewWith({ connection: offline, screen: 'offline' }));
      expect(screen.queryByTestId('crew-offline-tried')).toBeNull();
    });

    it('names the time and the reason, and changes on every attempt', () => {
      const view = renderWithCrew(
        <OnboardingScreen />,
        crewWith({ connection: offline, screen: 'offline' })
      );
      const first = new Date(2026, 8, 25, 9, 41, 7).getTime();
      attempt(view, first, 'unreachable');
      const line = screen.getByTestId('crew-offline-tried');
      expect(line).toHaveTextContent(
        `Tried again at ${attemptTime(first)}. Couldn’t reach hpc.ucsf.edu. ${emptyCopy.keepsTrying}`
      );
      // Connect hears it too: focus lands back on it after the attempt.
      expect(
        screen.getByRole('button', { name: emptyCopy.offlineAction('lab') })
      ).toHaveAccessibleDescription(line.textContent ?? '');

      // A repeat click 20 seconds later that fails the same way visibly did something.
      const second = first + 20_000;
      attempt(view, second, 'unreachable');
      expect(screen.getByTestId('crew-offline-tried')).toHaveTextContent(
        `Tried again at ${attemptTime(second)}.`
      );
      expect(attemptTime(second)).not.toBe(attemptTime(first));
    });

    it('says Crew keeps trying only for a failure the daemon retries', () => {
      const view = renderWithCrew(
        <OnboardingScreen />,
        crewWith({ connection: offline, screen: 'offline' })
      );
      attempt(view, Date.now(), 'ssh_failed');
      expect(screen.getByTestId('crew-offline-tried')).toHaveTextContent(emptyCopy.keepsTrying);

      // A sign-in or host-key failure is final until a person acts: never "keeps trying".
      const base = view.crew();
      cleanup();
      for (const kind of ['auth_required', 'host_key_unknown', 'unknown'] as const) {
        renderWithCrew(<OfflineState />, {
          ...base,
          lastConnectFailure: { kind, message: 'Crew SSH failure' },
        });
        const line = screen.getByTestId('crew-offline-tried');
        expect(line).toHaveTextContent(/^Tried again at /);
        expect(line).not.toHaveTextContent(emptyCopy.keepsTrying);
        cleanup();
      }
    });

    it('reports an attempt that failed while the offline screen stayed', async () => {
      const crew = crewWith({
        connection: offline,
        lastConnectFailure: { kind: 'unreachable', message: 'Crew SSH failure' },
      });
      renderWithCrew(<OfflineState />, crew);
      fireEvent.click(screen.getByRole('button', { name: emptyCopy.offlineAction('lab') }));
      expect(
        await screen.findByText(new RegExp(`Couldn’t reach hpc.ucsf.edu`))
      ).toBeInTheDocument();
    });
  });

  it('offers Sign in when the server wants a password', () => {
    const crew = crewWith();
    renderWithCrew(<SignInNeededState />, crew);
    expect(
      screen.getByRole('heading', { name: emptyCopy.signInTitle('hpc.ucsf.edu') })
    ).toBeInTheDocument();
    expect(screen.getByText(emptyCopy.signInBody)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.signInAction }));
    expect(crew.openSignIn).toHaveBeenCalled();
  });
});

describe('NotSetUpPane', () => {
  it('hands over a message for the host and keeps the install behind a disclosure', async () => {
    updateJoinContext('conn-1', { hostUsername: 'alice', hostDisplayName: 'Alice Chen' });
    const crew = crewWith({
      lastConnectFailure: { kind: 'bridge_missing', message: 'exit_127' },
    });
    renderWithCrew(<NotSetUpPane />, crew);

    expect(
      screen.getByRole('heading', { name: notSetUpCopy.title('hpc.ucsf.edu') })
    ).toBeInTheDocument();
    expect(screen.getByText(notSetUpCopy.body)).toBeInTheDocument();
    const message =
      'Hi Alice, Crew isn’t set up for my account (@bob) on hpc.ucsf.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?';
    expect(screen.getByText(message)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: `Copy ${notSetUpCopy.messageLabel}` }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(message));

    expect(screen.queryByText(/install -m 0755/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.installYourself }));
    fireEvent.click(screen.getByRole('button', { name: `Copy ${notSetUpCopy.installLabel}` }));
    await waitFor(() =>
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(INSTALL_COMMANDS)
    );

    fireEvent.click(screen.getByRole('button', { name: notSetUpCopy.tryAgain }));
    expect(crew.connect).toHaveBeenCalledWith({ userInitiated: true });
    expect(crew.registerErrorSlot).toHaveBeenCalledWith('connect');
  });

  it('still reads as a message without a known host', () => {
    renderWithCrew(
      <NotSetUpPane />,
      crewWith({ lastConnectFailure: { kind: 'handoff_failed', message: 'handoff' } })
    );
    expect(
      screen.getByText(
        'Crew isn’t set up for my account (@bob) on hpc.ucsf.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?'
      )
    ).toBeInTheDocument();
  });
});

describe('NoTeamState', () => {
  it('tells a member whom to ask, with Create a team as the quiet way on', () => {
    const crew = crewWith({ snapshot: fakeSnapshot(), screen: 'no-team' });
    renderWithCrew(<NoTeamState />, crew);
    expect(screen.getByRole('heading', { name: emptyCopy.memberTitle('lab') })).toBeInTheDocument();
    expect(screen.getByText(emptyCopy.memberBody('Alice Chen (@alice)'))).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.memberAction }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'create-team' });
  });

  it('offers to join the team a person is invited to', () => {
    const snapshot = fakeSnapshot({
      invitations: [
        {
          id: 'inv-1',
          kind: 'team',
          target_id: 'team-1',
          principal_id: 'p-bob',
          inviter_id: 'p-alice',
          expires_at: 0,
          target_name: 'Analysis Lab',
          inviter: { username: 'alice', display_name: 'Alice Chen' },
        },
      ],
    });
    const crew = crewWith({ snapshot, screen: 'no-team' });
    renderWithCrew(<NoTeamState />, crew);
    expect(
      screen.getByRole('heading', { name: emptyCopy.invitedTitle('Analysis Lab') })
    ).toBeInTheDocument();
    expect(screen.getByText(emptyCopy.invitedBody('Alice Chen (@alice)'))).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/inv-1|team-1|p-alice/);
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.invitedAction('Analysis Lab') }));
    expect(crew.mutate).toHaveBeenCalledWith('invitation.accept', { invitation_id: 'inv-1' });
  });

  describe('focus after joining the team', () => {
    const invited = () =>
      fakeSnapshot({
        invitations: [
          {
            id: 'inv-1',
            kind: 'team',
            target_id: 'team-1',
            principal_id: 'p-bob',
            inviter_id: 'p-alice',
            expires_at: 0,
            target_name: 'Analysis Lab',
            inviter: { username: 'alice', display_name: 'Alice Chen' },
          },
        ],
      });
    let composer: HTMLTextAreaElement | null = null;
    afterEach(() => {
      composer?.remove();
      composer = null;
    });
    const mountComposer = () => {
      composer = document.createElement('textarea');
      composer.setAttribute('aria-label', 'Message #general');
      document.body.appendChild(composer);
      return composer;
    };

    it('lands in the channel’s composer once it opens, instead of on the page (T-15)', async () => {
      const view = renderWithCrew(
        <NoTeamState />,
        crewWith({ snapshot: invited(), screen: 'no-team' })
      );
      const join = screen.getByRole('button', { name: emptyCopy.invitedAction('Analysis Lab') });
      join.focus();
      fireEvent.click(join);
      await waitFor(() => expect(view.crew().mutate).toHaveBeenCalled());
      // The card goes as the team's channel replaces it; the composer mounts a moment later.
      view.unmount();
      expect(document.activeElement).toBe(document.body);
      const box = await act(async () => mountComposer());
      await waitFor(() => expect(box).toHaveFocus());
    });

    it('leaves focus where the person put it meanwhile', async () => {
      const view = renderWithCrew(
        <NoTeamState />,
        crewWith({ snapshot: invited(), screen: 'no-team' })
      );
      fireEvent.click(
        screen.getByRole('button', { name: emptyCopy.invitedAction('Analysis Lab') })
      );
      await waitFor(() => expect(view.crew().mutate).toHaveBeenCalled());
      view.unmount();
      const elsewhere = document.createElement('button');
      document.body.appendChild(elsewhere);
      elsewhere.focus();
      const box = await act(async () => mountComposer());
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(box).not.toHaveFocus();
      expect(elsewhere).toHaveFocus();
      elsewhere.remove();
    });

    it('moves nothing when joining failed', async () => {
      const mutate = vi.fn().mockRejectedValue(new Error('refused'));
      const view = renderWithCrew(
        <NoTeamState />,
        crewWith({ snapshot: invited(), screen: 'no-team', mutate })
      );
      fireEvent.click(
        screen.getByRole('button', { name: emptyCopy.invitedAction('Analysis Lab') })
      );
      await waitFor(() => expect(mutate).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 0));
      view.unmount();
      const box = await act(async () => mountComposer());
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(box).not.toHaveFocus();
    });

    it('focuses a composer that is already there', () => {
      const box = mountComposer();
      const stop = focusComposerOnceMounted(null);
      expect(box).toHaveFocus();
      stop();
    });
  });

  it('shows the host the setup checklist', () => {
    const snapshot = fakeSnapshot({
      actor: { id: 'p-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' },
    });
    renderWithCrew(<NoTeamState />, crewWith({ snapshot, isHost: true, screen: 'no-team' }));
    expect(screen.getByRole('heading', { name: checklistCopy.title('lab') })).toBeInTheDocument();
    // It is the host's whole screen here, so it cannot be hidden.
    expect(screen.queryByRole('button', { name: checklistCopy.hide })).toBeNull();
  });

  it('enables nothing from the last verified view while it re-verifies', () => {
    const snapshot = fakeSnapshot();
    renderWithCrew(
      <NoTeamState />,
      crewWith({
        snapshot: null,
        lastVerified: {
          connectionId: 'conn-1',
          snapshot,
          observedPrivacy: {
            connectionId: 'conn-1',
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
    expect(screen.getByRole('button', { name: emptyCopy.memberAction })).toBeDisabled();
  });
});

describe('NoChannelState', () => {
  it('offers Create channel in a team with no open channel', () => {
    const snapshot = fakeSnapshot({
      teams: [
        {
          id: 'team-1',
          name: 'Analysis Lab',
          created_by: 'p-alice',
          members: ['p-alice', 'p-bob'],
          general_channel_id: 'c-1',
        },
      ],
    });
    const crew = crewWith({ snapshot, teamId: 'team-1', screen: 'no-channel' });
    renderWithCrew(<NoChannelState />, crew);
    expect(
      screen.getByRole('heading', { name: emptyCopy.noChannelTitle('Analysis Lab') })
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: emptyCopy.noChannelAction }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'create-channel', teamId: 'team-1' });
  });
});

describe('SetupChecklist', () => {
  const hostSnapshot = () =>
    fakeSnapshot({
      workspace: {
        id: 'workspace-1',
        host_uid: 1000,
        mode: 'private',
        institution_id: null,
        policy_epoch: 1,
        host_principal_id: 'p-alice',
        name: 'lab',
      },
      actor: { id: 'p-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' },
      principals: [{ id: 'p-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' }],
    });

  it('lists the three steps and opens the dialog that asks for each', () => {
    const crew = crewWith({ snapshot: hostSnapshot(), isHost: true });
    renderWithCrew(<SetupChecklist />, crew);
    expect(screen.getAllByRole('listitem')).toHaveLength(3);

    fireEvent.click(screen.getByRole('button', { name: checklistCopy.setInstitution('ucsf') }));
    expect(crew.openDialog).toHaveBeenCalledWith({
      kind: 'confirm',
      confirm: { action: 'set-institution', institutionId: 'ucsf' },
    });
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.createTeam }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'create-team' });
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.invitePeople }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'invite-people' });
  });

  it('ticks what is done and sends a missing institution to Connection settings', () => {
    const snapshot = hostSnapshot();
    snapshot.teams = [
      {
        id: 'team-1',
        name: 'Analysis Lab',
        created_by: 'p-alice',
        members: ['p-alice'],
        general_channel_id: 'c-1',
      },
    ];
    const crew = crewWith({
      snapshot,
      isHost: true,
      connection: fakeConnection({ institution_id: null }),
    });
    renderWithCrew(<SetupChecklist />, crew);
    const rows = screen.getAllByRole('listitem');
    expect(rows[1]).toHaveAttribute('data-done', 'true');
    expect(rows[0]).toHaveAttribute('data-done', 'false');
    expect(screen.getByText(checklistCopy.institutionMissing)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.connectionSettings }));
    expect(crew.openDialog).toHaveBeenCalledWith({
      kind: 'connection-settings',
      connectionId: 'conn-1',
    });
  });

  it('keeps "Invite people to {workspace}…" in reach of a channel while the host is alone (T-22)', () => {
    const crew = crewWith({ snapshot: hostSnapshot(), isHost: true });
    renderWithCrew(<SetupChecklist compact />, crew);
    const nudge = screen.getByTestId('crew-setup-invite-nudge');
    expect(nudge).toHaveTextContent(checklistCopy.aloneTitle('lab'));
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.invitePeopleTo('lab') }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'invite-people' });
    // One line, not the whole checklist.
    expect(screen.queryByTestId('crew-setup-checklist')).toBeNull();
  });

  it('drops the compact invite once someone else is in, or asks to be', () => {
    const joined = hostSnapshot();
    joined.principals = [
      ...joined.principals,
      { id: 'p-bob', uid: 1001, username: 'bob', nickname: 'bob' },
    ];
    const view = renderWithCrew(
      <SetupChecklist compact />,
      crewWith({ snapshot: joined, isHost: true })
    );
    expect(screen.queryByTestId('crew-setup-invite-nudge')).toBeNull();
    view.unmount();

    renderWithCrew(
      <SetupChecklist compact />,
      crewWith({ snapshot: hostSnapshot(), isHost: false })
    );
    expect(screen.queryByTestId('crew-setup-invite-nudge')).toBeNull();
  });

  it('is inside the channel the host opens, not only on the empty workspace (T-22)', () => {
    const snapshot = hostSnapshot();
    const channel: Channel = {
      id: 'c-general',
      team_id: 'team-1',
      name: 'general',
      created_by: 'p-alice',
      owner_id: 'p-alice',
      members: ['p-alice'],
      archived: false,
      classification: 'public_safe',
    };
    const intro = (crew: CrewController) =>
      renderWithCrew(
        <ChannelIntro
          channel={channel}
          viewerId="p-alice"
          dir={buildPeopleDirectory(snapshot)}
          readOnly={!crew.snapshot}
        />,
        crew
      );

    const host = crewWith({ snapshot, isHost: true });
    const view = intro(host);
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.invitePeopleTo('lab') }));
    expect(host.openDialog).toHaveBeenCalledWith({ kind: 'invite-people' });
    view.unmount();

    // While Crew re-verifies, the line stays but acts on nothing, like the rest of the timeline.
    const reverifying = intro(
      crewWith({
        snapshot: null,
        isHost: true,
        lastVerified: {
          connectionId: 'conn-1',
          snapshot,
          observedPrivacy: {
            connectionId: 'conn-1',
            mode: 'private',
            institutionId: null,
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: 'team-1',
          channelId: channel.id,
          messages: [],
        },
      })
    );
    expect(
      screen.getByRole('button', { name: checklistCopy.invitePeopleTo('lab') })
    ).toBeDisabled();
    reverifying.unmount();

    intro(crewWith({ snapshot, isHost: false }));
    expect(screen.queryByTestId('crew-setup-invite-nudge')).toBeNull();
  });

  it('hides on this computer, and never shows to a member', () => {
    const view = renderWithCrew(
      <SetupChecklist />,
      crewWith({ snapshot: hostSnapshot(), isHost: true })
    );
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.hide }));
    expect(screen.queryByTestId('crew-setup-checklist')).toBeNull();
    view.unmount();

    renderWithCrew(<SetupChecklist />, crewWith({ snapshot: hostSnapshot(), isHost: true }));
    expect(screen.queryByTestId('crew-setup-checklist')).toBeNull();
    localStorage.clear();

    renderWithCrew(<SetupChecklist />, crewWith({ snapshot: hostSnapshot(), isHost: false }));
    expect(screen.queryByTestId('crew-setup-checklist')).toBeNull();
  });
});

describe('OnboardingScreen', () => {
  it('draws the screen the controller derived, and nothing for the layout’s own screens', () => {
    const view = renderWithCrew(<OnboardingScreen />, makeCrew({ screen: 'welcome' }));
    expect(screen.getByRole('heading', { name: welcomeCopy.title })).toBeInTheDocument();
    expect(ONBOARDING_SCREENS).not.toContain('channel');

    view.update({ screen: 'channel' });
    expect(screen.queryByRole('heading')).toBeNull();
    view.update({ screen: 'loading' });
    expect(screen.queryByRole('heading')).toBeNull();
  });
});
