import { describe, expect, it } from 'vitest';
import {
  channelName,
  channelNamesAcrossTeams,
  channelSlug,
  connectionNames,
  connectionServer,
  teamName,
  workspaceName,
} from './objectNames';
import { joinerPerson } from './personLabel';
import { buildPeopleDirectory } from './usePeopleDirectory';

const UUID = '3f2a9c1e-77b0-4d4e-9a1b-2c3d4e5f6a7b';

describe('teams', () => {
  it('render as typed, never upper-cased', () => {
    expect(teamName({ id: 't1', name: 'Analysis Lab' })).toBe('Analysis Lab');
    expect(teamName({ id: 't1', name: 'analysis lab' })).toBe('analysis lab');
  });

  it('prefer the projected display name, and never fall back to an ID', () => {
    expect(teamName({ id: 't1', name: 'raw', display_name: 'Analysis Lab' })).toBe('Analysis Lab');
    expect(teamName({ id: UUID, name: '   ' })).toBe('Untitled team');
    expect(teamName({ id: UUID, name: UUID })).toBe('Untitled team');
    expect(teamName(null)).toBe('Untitled team');
  });
});

describe('channels', () => {
  it('render as #slug', () => {
    expect(channelName({ id: 'c1', name: 'methods' })).toBe('#methods');
    expect(channelName({ id: 'c1', name: '#methods' })).toBe('#methods');
    expect(channelSlug({ id: 'c1', name: 'methods' })).toBe('methods');
  });

  it('never fall back to an ID', () => {
    expect(channelName({ id: UUID, name: 'ab'.repeat(32) })).toBe('#untitled');
    expect(channelName({ id: UUID, name: '' })).toBe('#untitled');
  });

  it('qualify a slug two teams share, and only that slug, in a list that spans teams', () => {
    const teams = [
      { id: 't1', name: 'Analysis Lab' },
      { id: 't2', name: 'Imaging Core' },
    ];
    const labels = channelNamesAcrossTeams(
      [
        { id: 'c1', team_id: 't1', name: 'methods' },
        { id: 'c2', team_id: 't2', name: 'Methods' },
        { id: 'c3', team_id: 't1', name: 'general' },
        { id: 'c4', team_id: 't2', name: 'scans' },
      ],
      teams
    );
    expect(labels.get('c1')).toBe('Analysis Lab / #methods');
    expect(labels.get('c2')).toBe('Imaging Core / #Methods');
    expect(labels.get('c3')).toBe('#general');
    expect(labels.get('c4')).toBe('#scans');
  });

  it('do not qualify a slug that repeats within one team', () => {
    const labels = channelNamesAcrossTeams(
      [
        { id: 'c1', team_id: 't1', name: 'general' },
        { id: 'c2', team_id: 't1', name: 'general' },
      ],
      [{ id: 't1', name: 'Analysis Lab' }]
    );
    expect(labels.get('c1')).toBe('#general');
    expect(labels.get('c2')).toBe('#general');
  });
});

describe('workspaces', () => {
  const dir = buildPeopleDirectory({
    workspace: { host_uid: 1000 },
    actor: { id: 'p1', username: 'alice', nickname: 'Alice Chen', uid: 1000 },
    principals: [],
  });

  it('render their name', () => {
    expect(workspaceName({ name: 'lab' }, dir.host)).toBe('lab');
  });

  it("render a legacy unnamed workspace as its host's", () => {
    expect(workspaceName({}, dir.host)).toBe("Alice Chen's workspace");
    expect(workspaceName({ name: null }, joinerPerson('bob'))).toBe("bob's workspace");
  });

  it('never fall back to an ID', () => {
    expect(workspaceName({ name: UUID }, null)).toBe('Unnamed workspace');
    expect(workspaceName(null, null)).toBe('Unnamed workspace');
  });
});

describe('saved connections', () => {
  it('read the server from an SSH target', () => {
    expect(connectionServer({ id: 'x', ssh_target: 'alice@hpc.ucsf.edu' })).toBe('hpc.ucsf.edu');
    expect(connectionServer({ id: 'x', ssh_target: 'hpc' })).toBe('hpc');
  });

  it('are labelled "name — server" only when two share a name', () => {
    const labels = connectionNames([
      { id: 'a', name: 'lab', ssh_target: 'alice@hpc.ucsf.edu' },
      { id: 'b', name: 'Lab', ssh_target: 'alice@sdsc.edu' },
      { id: 'c', name: 'imaging-core', ssh_target: 'alice@hpc.ucsf.edu' },
      { id: 'd', name: '', ssh_target: 'bob@cluster.example.org' },
    ]);
    expect(labels.get('a')).toBe('lab — hpc.ucsf.edu');
    expect(labels.get('b')).toBe('Lab — sdsc.edu');
    expect(labels.get('c')).toBe('imaging-core');
    expect(labels.get('d')).toBe('cluster.example.org');
  });

  it('never show an ID', () => {
    const labels = connectionNames([{ id: UUID, name: UUID, ssh_target: '' }]);
    expect(labels.get(UUID)).toBe('Unnamed workspace');
  });
});
