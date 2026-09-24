import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createChannelCopy, createTeamCopy, nameRuleCopy } from './copy';
import { CreateChannelDialog } from './CreateChannelDialog';
import { CreateTeamDialog } from './CreateTeamDialog';
import { CrewDialogs } from './CrewDialogs';
import {
  alice,
  bob,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { RenameDialog } from './RenameDialog';

const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', () => toasts);

installResizeObserverStub();

afterEach(() => vi.clearAllMocks());

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
        throw new Error('name_conflict: taken');
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

describe('CreateTeamDialog', () => {
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

  it('says a taken team name in the S2 wording', async () => {
    renderWithCrew(<CreateTeamDialog onClose={vi.fn()} />, {
      request: () => {
        throw new Error('name_conflict: a team with this name exists');
      },
    });
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Analysis Lab' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create team' }));
    });
    expect(await screen.findByText(nameRuleCopy.teamTaken)).toBeInTheDocument();
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
