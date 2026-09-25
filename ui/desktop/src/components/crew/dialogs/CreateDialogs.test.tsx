import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CrewControllerProvider, useCrew } from '../state/CrewControllerContext';
import { addPeopleCopy, createChannelCopy, createTeamCopy, nameRuleCopy } from './copy';
import { CreateChannelDialog, examplePlaceholder, withoutLeadingHash } from './CreateChannelDialog';
import { CreateTeamDialog, teamExamplePlaceholder } from './CreateTeamDialog';
import { CrewDialogs } from './CrewDialogs';
import {
  alice,
  bob,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { ANNOUNCE_DELAY_MS } from './fields';
import { DIRECT_ADD_CAPABILITY } from './people';
import { RenameDialog } from './RenameDialog';

const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', () => toasts);

installResizeObserverStub();

afterEach(() => vi.clearAllMocks());

/** The broker's literal refusals (`broker.rs`, `TEAM_NAME_TAKEN` and `CHANNEL_NAME_TAKEN`). */
const TEAM_TAKEN =
  'name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.';
const CHANNEL_TAKEN =
  'name_taken: A channel with this name, or one that looks like it, already exists in this team. Choose a different name.';

describe('CreateChannelDialog', () => {
  it('previews the slug the broker will store, with Content visible and Restricted by default', async () => {
    renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    const dialog = await screen.findByRole('dialog', { name: 'Create channel' });
    expect(dialog).toHaveTextContent(createChannelCopy.inTeam('Analysis Lab'));
    const name = screen.getByLabelText('Name');
    await waitFor(() => expect(name).toHaveFocus());
    expect(screen.getByRole('radio', { name: /^Restricted/ })).toBeChecked();
    expect(screen.getByRole('radio', { name: /^Public-safe/ })).not.toBeChecked();
    expect(screen.getByText(nameRuleCopy.consequence)).toBeInTheDocument();

    fireEvent.change(name, { target: { value: '  #Data Analysis.v2 ' } });
    expect(screen.getByText('Will be created as #data-analysis-v2')).toBeInTheDocument();
  });

  // QA Q2-31: "e.g. methods" sat beside an existing #methods, reading as a nudge to duplicate it.
  it('gives an example name that is never one the team already has', async () => {
    renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    expect(await screen.findByLabelText('Name')).toHaveAttribute(
      'placeholder',
      'e.g. journal-club'
    );
    const base = makeSnapshot().channels[0];
    const journalClub = { ...base, id: 'channel-jc', name: 'journal-club' };
    expect(examplePlaceholder([base, journalClub], 'team-1')).toBe('e.g. new-channel');
    // Another team's #journal-club is no reason to change this team's example.
    expect(examplePlaceholder([base, { ...journalClub, team_id: 'team-2' }], 'team-1')).toBe(
      'e.g. journal-club'
    );
    expect(
      examplePlaceholder(
        [{ ...journalClub, name: 'Journal Club', handle: 'journal-club' }],
        'team-1'
      )
    ).toBe('e.g. new-channel');
  });

  it('creates the channel with the previewed slug and selects it', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={onClose} />, {
      request: (method) => (method === 'channel.create' ? { id: 'channel-new' } : {}),
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Raw Data' } });
    fireEvent.click(screen.getByRole('radio', { name: /^Public-safe/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.create')).toEqual([
        { team_id: 'team-1', name: 'raw-data', classification: 'public_safe' },
      ])
    );
    expect(crew.selectChannel).toHaveBeenCalledWith('channel-new');
    expect(onClose).toHaveBeenCalled();
  });

  it('refuses a name the broker would refuse before sending it', async () => {
    const { crew } = renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    const name = await screen.findByLabelText('Name');
    fireEvent.change(name, { target: { value: 'results/final' } });
    fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    expect(name).toBeInvalid();
    expect(await screen.findByText(nameRuleCopy.channelReserved)).toBeInTheDocument();
    expect(requestsFor(crew, 'channel.create')).toEqual([]);
  });

  it('shows the S2 refusal on the name, in its exact words', async () => {
    renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />, {
      request: () => {
        throw new CrewHttpError(CHANNEL_TAKEN, 400, 'crew_request_refused');
      },
    });
    const name = await screen.findByLabelText('Name');
    fireEvent.change(name, { target: { value: 'methods' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    expect(await screen.findByText(nameRuleCopy.channelTaken)).toBeInTheDocument();
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('describes the name with its consequence line, and announces a problem once typing pauses', async () => {
    renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    const name = await screen.findByLabelText('Name');
    // QA T-72: the consequence line was on screen but not linked to the field.
    expect(name).toHaveAccessibleDescription(nameRuleCopy.consequence);
    fireEvent.change(name, { target: { value: 'methods' } });
    expect(name).toHaveAccessibleDescription(
      `${createChannelCopy.preview('methods')} ${nameRuleCopy.consequence}`
    );

    vi.useFakeTimers();
    try {
      fireEvent.change(name, { target: { value: 'results/final' } });
      const region = screen.getByRole('status');
      expect(region).toHaveAttribute('aria-live', 'polite');
      // Not on every keystroke…
      expect(region).toHaveTextContent('');
      act(() => vi.advanceTimersByTime(ANNOUNCE_DELAY_MS));
      // …but once the person stops typing.
      expect(region).toHaveTextContent(nameRuleCopy.channelReserved);
      fireEvent.change(name, { target: { value: 'results' } });
      expect(region).toHaveTextContent('');
    } finally {
      vi.useRealTimers();
    }
  });

  it('forgets its own refusal when it closes, instead of passing it to the connection bar', async () => {
    // QA T-08: Cancel after a refusal left "name_taken: …" raw over the page.
    const { crew } = renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'create-channel', teamId: 'team-1' },
      request: () => {
        throw new CrewHttpError(CHANNEL_TAKEN, 400, 'crew_request_refused');
      },
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'general' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    expect(await screen.findByText(nameRuleCopy.channelTaken)).toBeInTheDocument();
    expect(crew.current().error?.source).toBe('dialog:create-channel');

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(crew.current().error).toBeNull());
  });

  it('leaves an error from another surface alone when it closes', async () => {
    const { crew } = renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'create-channel', teamId: 'team-1' },
    });
    await screen.findByLabelText('Name');
    act(() => crew.current().reportError('Crew updates stopped.', 'global'));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await act(async () => {});
    expect(crew.current().error).toEqual({ message: 'Crew updates stopped.', source: 'global' });
  });

  it('starts clean every time it opens (L14)', async () => {
    const { crew } = renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'create-channel', teamId: 'team-1' },
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'methods' } });
    fireEvent.click(screen.getByRole('radio', { name: /^Public-safe/ }));
    act(() => crew.current().closeDialog());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    act(() => crew.current().openDialog({ kind: 'create-channel', teamId: 'team-1' }));
    expect(await screen.findByLabelText('Name')).toHaveValue('');
    expect(screen.getByRole('radio', { name: /^Restricted/ })).toBeChecked();
  });
});

describe('CreateChannelDialog, the name field (QA Q3-38)', () => {
  it('drops a typed or pasted leading # from the value it shows, since the field shows one', async () => {
    const { crew } = renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />, {
      request: (method) => (method === 'channel.create' ? { id: 'channel-new' } : {}),
    });
    const name = await screen.findByLabelText('Name');
    fireEvent.change(name, { target: { value: '#data' } });
    // "# #data" was the adornment plus a typed #.
    expect(name).toHaveValue('data');
    expect(screen.getByText(createChannelCopy.preview('data'))).toBeInTheDocument();
    fireEvent.change(name, { target: { value: ' ##data' } });
    expect(name).toHaveValue('data');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.create')).toEqual([
        { team_id: 'team-1', name: 'data', classification: 'restricted' },
      ])
    );
    // Only a LEADING #: anything else is the name rules' to refuse, in their own words.
    expect(withoutLeadingHash('da#ta')).toBe('da#ta');
    expect(withoutLeadingHash('＃data')).toBe('data');
  });

  it('says what a taken name reveals in a lab’s words, not "identifiers"', async () => {
    renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    const line = await screen.findByText(nameRuleCopy.consequence);
    expect(line).toHaveTextContent(
      'Everyone in this team can see whether a name is taken, so don’t put patient or sample IDs in channel names.'
    );
    expect(line.textContent).not.toMatch(/identifiers/i);
  });

  it('says an empty name under the field, never in the browser’s bubble', async () => {
    const { crew } = renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
    const name = await screen.findByLabelText('Name');
    expect(name.closest('form')).toHaveAttribute('novalidate');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    expect(await screen.findByText(nameRuleCopy.channelEmpty)).toBeInTheDocument();
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(name).toHaveAccessibleDescription(
      `${nameRuleCopy.channelEmpty} ${nameRuleCopy.consequence}`
    );
    expect(name).toHaveFocus();
    expect(requestsFor(crew, 'channel.create')).toEqual([]);

    // Spaces are no name either.
    fireEvent.input(name, { target: { value: '   ' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
    });
    expect(screen.getByText(nameRuleCopy.channelEmpty)).toBeInTheDocument();
    expect(requestsFor(crew, 'channel.create')).toEqual([]);
  });

  /**
   * After a Create press has said something under the field, each keystroke is judged as typed
   * (QA Q3-38). Every step is ONE `input` event, as a browser sends per keystroke: the form's own
   * `onInput` reads `validity` before the keystroke's problem reaches the field, so a test that
   * fires a second event for the same value hides a message that is one keystroke stale.
   */
  describe('after a Create press, one keystroke at a time', () => {
    async function pressCreateWith(value: string) {
      const view = renderWithCrew(<CreateChannelDialog teamId="team-1" onClose={vi.fn()} />);
      const name = await screen.findByLabelText('Name');
      fireEvent.input(name, { target: { value } });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Create channel' }));
      });
      return { ...view, name };
    }

    async function type(name: HTMLElement, value: string) {
      await act(async () => {
        fireEvent.input(name, { target: { value } });
      });
    }

    it('clears "can’t be empty" on the first letter typed after spaces', async () => {
      const { name } = await pressCreateWith('   ');
      expect(screen.getByText(nameRuleCopy.channelEmpty)).toBeInTheDocument();
      await type(name, '   d');
      expect(name).toHaveValue('   d');
      expect(screen.queryByText(nameRuleCopy.channelEmpty)).toBeNull();
      expect(name).not.toHaveAttribute('aria-invalid');
      expect(name).toHaveAccessibleDescription(
        `${createChannelCopy.preview('d')} ${nameRuleCopy.consequence}`
      );
    });

    it('clears a reserved character’s message when that character is deleted', async () => {
      const { name } = await pressCreateWith('a/b');
      expect(screen.getByText(nameRuleCopy.channelReserved)).toBeInTheDocument();
      await type(name, 'ab');
      expect(screen.queryByText(nameRuleCopy.channelReserved)).toBeNull();
      expect(name).not.toHaveAttribute('aria-invalid');
    });

    it('says the problem the name has now, not the one it had', async () => {
      const { name, crew } = await pressCreateWith('   ');
      await type(name, '   /');
      expect(screen.getByText(nameRuleCopy.channelReserved)).toBeInTheDocument();
      expect(screen.queryByText(nameRuleCopy.channelEmpty)).toBeNull();
      expect(name).toHaveAttribute('aria-invalid', 'true');
      // Emptied again, it is empty again.
      await type(name, '');
      expect(screen.getByText(nameRuleCopy.channelEmpty)).toBeInTheDocument();
      expect(screen.queryByText(nameRuleCopy.channelReserved)).toBeNull();
      expect(requestsFor(crew, 'channel.create')).toEqual([]);
    });
  });
});

describe('CreateTeamDialog', () => {
  it('gives an example name that is never a team the workspace already has (QA Q3-38)', async () => {
    renderWithCrew(<CreateTeamDialog onClose={vi.fn()} />);
    const name = await screen.findByLabelText('Name');
    expect(name).toHaveAttribute('placeholder', 'e.g. Imaging Group');
    // The harness's workspace has Analysis Lab, the example this used to give.
    expect(name.getAttribute('placeholder')).not.toContain('Analysis Lab');
    expect(teamExamplePlaceholder([{ name: 'Analysis Lab' }])).toBe('e.g. Imaging Group');
    expect(teamExamplePlaceholder([{ name: 'Imaging Group' }])).toBe('e.g. new-team');
    expect(teamExamplePlaceholder([{ name: 'Imaging', handle: 'imaging-group' }])).toBe(
      'e.g. new-team'
    );
  });

  it('creates a team, offers the Add people step, and lands on the new team', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(<CreateTeamDialog onClose={onClose} />, {
      request: (method) =>
        method === 'team.create'
          ? { team: { id: 'team-new', name: 'Imaging Core' }, channel: { id: 'c' } }
          : {},
    });
    const dialog = await screen.findByRole('dialog', { name: 'Create team' });
    expect(dialog).toHaveTextContent(createTeamCopy.helper('lab'));
    const name = screen.getByLabelText('Name');
    expect(name).toHaveAccessibleDescription(createTeamCopy.helper('lab'));
    await waitFor(() => expect(name).toHaveFocus());
    fireEvent.change(name, { target: { value: 'Imaging Core' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    expect(requestsFor(crew, 'team.create')).toEqual([{ name: 'Imaging Core' }]);

    expect(
      await screen.findByRole('dialog', { name: 'Add people to Imaging Core' })
    ).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: /^Person/ })).toHaveFocus());
    fireEvent.click(screen.getByRole('button', { name: /^Person/ }));
    fireEvent.click(await screen.findByRole('option', { name: /Bob Lee/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'invitation.create')).toEqual([
        { kind: 'team', target_id: 'team-new', principal_id: bob.id, expected_username: 'bob' },
      ])
    );
    expect(toasts.toastSuccess).toHaveBeenCalledWith({
      msg: 'Invited. Bob Lee (@bob) will see it in Crew and needs to accept.',
    });
    expect(crew.selectTeam).toHaveBeenCalledWith('team-new');
    expect(onClose).toHaveBeenCalled();
  });

  it('adds the person straight into the new team when the broker adds directly', async () => {
    function WithDirectAdd({ children }: { children: ReactNode }) {
      const crew = useCrew();
      return (
        <CrewControllerProvider controller={{ ...crew, capabilities: [DIRECT_ADD_CAPABILITY] }}>
          {children}
        </CrewControllerProvider>
      );
    }
    const onClose = vi.fn();
    const { crew } = renderWithCrew(
      <WithDirectAdd>
        <CreateTeamDialog onClose={onClose} />
      </WithDirectAdd>,
      {
        request: (method) =>
          method === 'team.create'
            ? {
                team: { id: 'team-new', name: 'Imaging Core' },
                channel: { id: 'channel-new-general', name: 'general' },
              }
            : { team_id: 'team-new', principal_id: bob.id, added_channels: [] },
      }
    );
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Imaging Core' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    fireEvent.click(await screen.findByRole('button', { name: /^Person/ }));
    fireEvent.click(await screen.findByRole('option', { name: /Bob Lee/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'team.add_member')).toEqual([
        { team_id: 'team-new', principal_id: bob.id, expected_username: 'bob' },
      ])
    );
    expect(requestsFor(crew, 'invitation.create')).toEqual([]);
    expect(toasts.toastSuccess).toHaveBeenCalledWith({
      msg: addPeopleCopy.added('Bob Lee (@bob)', '#general'),
    });
    expect(crew.selectTeam).toHaveBeenCalledWith('team-new');
    expect(onClose).toHaveBeenCalled();
  });

  it('can skip adding people', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(<CreateTeamDialog onClose={onClose} />, {
      request: () => ({ team: { id: 'team-new', name: 'Imaging Core' } }),
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Imaging Core' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Skip for now' }));
    expect(requestsFor(crew, 'invitation.create')).toEqual([]);
    expect(crew.selectTeam).toHaveBeenCalledWith('team-new');
    expect(onClose).toHaveBeenCalled();
  });

  it('skips the Add people step when no one else has joined', async () => {
    const onClose = vi.fn();
    renderWithCrew(<CreateTeamDialog onClose={onClose} />, {
      snapshot: makeSnapshot({ principals: [alice] }),
      request: () => ({ team: { id: 'team-new', name: 'Solo' } }),
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Solo' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it.each([
    ['the daemon’s text', TEAM_TAKEN],
    [
      'an older daemon’s envelope',
      `Crew broker refused request: ${JSON.stringify({ code: 'name_taken', message: TEAM_TAKEN })}`,
    ],
  ])('says a taken team name in the S2 wording, from %s', async (_from, refusal) => {
    renderWithCrew(<CreateTeamDialog onClose={vi.fn()} />, {
      request: () => {
        throw new CrewHttpError(refusal, 400, 'crew_request_refused');
      },
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Analysis Lab' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    expect(await screen.findByText(nameRuleCopy.teamTaken)).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('Crew broker refused request');
  });

  it('says a rate limit in the broker’s sentence, not on the name field', async () => {
    renderWithCrew(<CreateTeamDialog onClose={vi.fn()} />, {
      request: () => {
        throw new CrewHttpError(
          'rate_limited: Too many name attempts. Try again later.',
          400,
          'crew_request_refused'
        );
      },
    });
    const name = await screen.findByLabelText('Name');
    fireEvent.change(name, { target: { value: 'Analysis Lab' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    expect(await screen.findByText('Too many name attempts. Try again later.')).toBeInTheDocument();
    expect(name).not.toHaveAttribute('aria-invalid', 'true');
  });
});

describe('RenameDialog', () => {
  it('renames a channel to its slug and a team to its typed name', async () => {
    const channel = renderWithCrew(
      <RenameDialog target="channel" targetId="channel-general" onClose={vi.fn()} />
    );
    expect(await screen.findByRole('dialog', { name: 'Rename channel' })).toBeInTheDocument();
    const name = screen.getByLabelText('Name');
    expect(name).toHaveValue('general');
    fireEvent.change(name, { target: { value: 'Lab Notes' } });
    expect(screen.getByText('Will be created as #lab-notes')).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    });
    await waitFor(() =>
      expect(requestsFor(channel.crew, 'channel.rename')).toEqual([
        { channel_id: 'channel-general', name: 'lab-notes' },
      ])
    );
    channel.unmount();

    const team = renderWithCrew(<RenameDialog target="team" targetId="team-1" onClose={vi.fn()} />);
    const teamName = await screen.findByLabelText('Name');
    expect(teamName).toHaveValue('Analysis Lab');
    fireEvent.change(teamName, { target: { value: 'Analysis Group' } });
    await act(async () => {
      fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Rename' }));
    });
    await waitFor(() =>
      expect(requestsFor(team.crew, 'team.rename')).toEqual([
        { team_id: 'team-1', name: 'Analysis Group' },
      ])
    );
  });

  it('holds a workspace name to the workspace rule', async () => {
    const { crew } = renderWithCrew(
      <RenameDialog target="workspace" targetId="workspace-1" onClose={vi.fn()} />
    );
    const name = await screen.findByLabelText('Name');
    expect(name).toHaveValue('lab');
    fireEvent.change(name, { target: { value: 'Lab-' } });
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    expect(name).toBeInvalid();
    expect(requestsFor(crew, 'workspace.rename')).toEqual([]);
    fireEvent.change(name, { target: { value: 'imaging-core' } });
    expect(name).toBeValid();
  });
});
