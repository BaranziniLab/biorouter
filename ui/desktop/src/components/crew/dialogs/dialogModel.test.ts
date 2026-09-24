import { describe, expect, it } from 'vitest';
import { buildPeopleDirectory, joinerPerson } from '../identity';
import { inviteCopy, letInCopy, nameRuleCopy, refusalCopy } from './copy';
import { groupedFingerprint, workspaceKeyFingerprint } from './fingerprint';
import { makeSnapshot, connection, bob } from './dialogsTestHarness';
import { enrollmentInviteFrom, legacyTokenFrom, parseJoinRequest } from './joinRequest';
import {
  channelSlugPreview,
  channelSlugProblem,
  INSTITUTION_FIELD_PATTERN,
  teamNameProblem,
  WORKSPACE_NAME_PATTERN,
  workspaceNameProblem,
} from './nameRules';
import { firstName, liveInvitations, personMatches } from './people';
import {
  approveRefusalText,
  inviteRefusal,
  isNameRefusal,
  nameRefusalText,
  parseRefusal,
  refusalText,
} from './refusals';
import { uniqueNamesSupported, workspaceLabelFor, workspacePhraseFor } from './workspace';

describe('name rules', () => {
  it.each([
    ['methods', 'methods'],
    ['#Methods', 'methods'],
    ['  Data Analysis ', 'data-analysis'],
    ['raw.data--v2', 'raw-data-v2'],
    ['Ｒaw Ｄata', 'raw-data'],
    ['lab_notes', 'lab_notes'],
    ['-edge-', 'edge'],
    ['#', ''],
  ])('previews %j as the slug %j', (raw, slug) => {
    expect(channelSlugPreview(raw)).toBe(slug);
  });

  it('catches the channel refusals worth catching before a round trip', () => {
    expect(channelSlugProblem('')).toBe(nameRuleCopy.channelEmpty);
    expect(channelSlugProblem('a/b')).toBe(nameRuleCopy.channelReserved);
    expect(channelSlugProblem('hello!')).toBe(nameRuleCopy.channelDisallowed);
    expect(channelSlugProblem('_notes')).toBe(nameRuleCopy.channelStart);
    expect(channelSlugProblem('x'.repeat(81))).toBe(nameRuleCopy.channelTooLong);
    expect(channelSlugProblem('11111111-2222-4333-8444-555555555555')).toBe(
      nameRuleCopy.channelLooksLikeId
    );
    expect(channelSlugProblem('méthodes')).toBeNull();
    expect(channelSlugProblem('lab_notes')).toBeNull();
  });

  it('refuses selector characters in a team name, lookalikes included', () => {
    expect(teamNameProblem('Analysis Lab')).toBeNull();
    expect(teamNameProblem('Lab @ UCSF')).not.toBeNull();
    expect(teamNameProblem('Lab ＃1')).not.toBeNull();
  });

  it('holds a workspace name to its ASCII rule', () => {
    expect(workspaceNameProblem('lab')).toBeNull();
    expect(workspaceNameProblem('imaging-core-2')).toBeNull();
    expect(workspaceNameProblem('Lab')).not.toBeNull();
    expect(workspaceNameProblem('lab-')).not.toBeNull();
    expect(workspaceNameProblem('x'.repeat(41))).not.toBeNull();
  });

  it('writes every pattern so the v flag browsers compile it with accepts it', () => {
    for (const pattern of [INSTITUTION_FIELD_PATTERN, WORKSPACE_NAME_PATTERN]) {
      expect(() => new RegExp(`^(?:${pattern})$`, 'v')).not.toThrow();
    }
    const institution = new RegExp(`^(?:${INSTITUTION_FIELD_PATTERN})$`, 'v');
    expect(institution.test('ucsf')).toBe(true);
    expect(institution.test('sdsc-west_2')).toBe(true);
    expect(institution.test('UCSF')).toBe(false);
    expect(institution.test('-ucsf')).toBe(false);
    expect(institution.test('x'.repeat(65))).toBe(false);
  });
});

describe('refusals', () => {
  const TEAM_TAKEN =
    'name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.';
  const CHANNEL_TAKEN =
    'name_taken: A channel with this name, or one that looks like it, already exists in this team. Choose a different name.';
  const envelope = (code: string, message: string) =>
    `Crew broker refused request: ${JSON.stringify({ code, message })}`;

  it('splits the broker’s code from its sentence', () => {
    expect(parseRefusal('forbidden: team owner required')).toEqual({
      code: 'forbidden',
      sentence: 'team owner required',
      text: 'forbidden: team owner required',
      raw: 'forbidden: team owner required',
    });
    expect(parseRefusal('Plain words.').code).toBeNull();
  });

  it('removes an older daemon’s envelope before reading the code', () => {
    const wrapped = envelope('forbidden', 'forbidden: team owner required');
    expect(parseRefusal(wrapped)).toEqual({
      code: 'forbidden',
      sentence: 'team owner required',
      text: 'forbidden: team owner required',
      raw: wrapped,
    });
  });

  it('shows a person-written sentence without its code, and anything else verbatim', () => {
    expect(refusalText('name_invalid: Team name can’t be empty.')).toBe(
      'Team name can’t be empty.'
    );
    expect(refusalText(envelope('name_invalid', 'name_invalid: Team name can’t be empty.'))).toBe(
      'Team name can’t be empty.'
    );
    expect(refusalText('forbidden: team owner required')).toBe('forbidden: team owner required');
    expect(refusalText(envelope('forbidden', 'forbidden: team owner required'))).toBe(
      'forbidden: team owner required'
    );
    expect(refusalText('')).toBe('Crew couldn’t complete that action.');
  });

  it('words a taken name the same whether or not its holder is visible', () => {
    expect(nameRefusalText(TEAM_TAKEN, 'team')).toBe(nameRuleCopy.teamTaken);
    expect(nameRefusalText(CHANNEL_TAKEN, 'channel')).toBe(nameRuleCopy.channelTaken);
    expect(nameRefusalText(envelope('name_taken', CHANNEL_TAKEN), 'channel')).toBe(
      nameRuleCopy.channelTaken
    );
    expect(isNameRefusal(TEAM_TAKEN)).toBe(true);
    expect(isNameRefusal(envelope('name_taken', TEAM_TAKEN))).toBe(true);
    expect(isNameRefusal('name_invalid: Team name can’t be empty.')).toBe(true);
    expect(isNameRefusal('forbidden: team owner required')).toBe(false);
  });

  it('maps the invite refusals the copy deck words, and nothing it does not recognize', () => {
    expect(
      inviteRefusal(
        'unknown_account: There is no account @zed on this server. Check the spelling.',
        'zed',
        'lab'
      )
    ).toEqual({
      text: inviteCopy.refusal.noAccount('zed'),
      alreadyMember: false,
    });
    expect(
      inviteRefusal(
        'already_member: @bob is already a member. Choose Add device to add another computer for them.',
        'bob',
        'lab'
      )
    ).toEqual({
      text: inviteCopy.refusal.alreadyMember('bob', 'lab'),
      alreadyMember: true,
    });
    expect(
      inviteRefusal(
        'identity_ambiguous: @Bob is an alias on this server. Invite @bob.',
        'Bob',
        'lab'
      ).text
    ).toBe(inviteCopy.refusal.canonical('bob'));
    expect(
      inviteRefusal(
        'identity_conflict: another active member is @bob; remove the old @bob first',
        'bob',
        'lab'
      ).text
    ).toBe(refusalCopy.identityConflict('bob'));
    expect(
      inviteRefusal(
        'identity_conflict: Another account on this server is already invited as @bob. Cancel that invitation first.',
        'bob',
        'lab'
      ).text
    ).toBe(
      'Another account on this server is already invited as @bob. Cancel that invitation first.'
    );
  });

  it('says a device was already let in at approval', () => {
    const approved =
      'already_approved: You already let a device in for @eve. Replace the code only if they sent you a new one.';
    expect(approveRefusalText(approved, 'eve')).toBe(letInCopy.alreadyApproved('eve'));
    expect(approveRefusalText(envelope('already_approved', approved), 'eve')).toBe(
      letInCopy.alreadyApproved('eve')
    );
    expect(approveRefusalText('forbidden: only a person can invite or admit people', 'eve')).toBe(
      'forbidden: only a person can invite or admit people'
    );
  });
});

describe('people', () => {
  const dir = buildPeopleDirectory(makeSnapshot());

  it('takes {first} from the display name, the server account, or the username', () => {
    expect(firstName(dir.byId(bob.id))).toBe('Bob');
    expect(firstName(joinerPerson('eve', 'Eve Park'))).toBe('Eve');
    expect(firstName(joinerPerson('eve'))).toBe('@eve');
  });

  it('matches a query against display names and usernames, ignoring a leading @', () => {
    const person = dir.byId(bob.id)!;
    expect(personMatches(person, 'lee')).toBe(true);
    expect(personMatches(person, '@bo')).toBe(true);
    expect(personMatches(person, 'carol')).toBe(false);
    expect(personMatches(person, '  ')).toBe(true);
  });

  it('drops expired invitations', () => {
    const base = { kind: 'team' as const, target_id: 't', principal_id: 'p', inviter_id: 'i' };
    expect(
      liveInvitations(
        [
          { ...base, id: 'a', expires_at: 200 },
          { ...base, id: 'b', expires_at: 50 },
          { ...base, id: 'c', expires_at: 200, expired: true },
        ],
        100
      ).map((invitation) => invitation.id)
    ).toEqual(['a']);
  });
});

describe('join requests and invite answers', () => {
  const key = 'ef'.repeat(32);

  it('takes exactly one device key, and the username when there is one', () => {
    expect(parseJoinRequest(`Username: @dana\nDevice key: ${key}`)).toEqual({
      publicKey: key,
      username: 'dana',
    });
    expect(parseJoinRequest(`${key.toUpperCase()}`)).toEqual({ publicKey: key, username: null });
    expect(parseJoinRequest('no key here')).toBeNull();
    expect(parseJoinRequest(`${key} ${'12'.repeat(32)}`)).toBeNull();
  });

  it('checks the broker’s answers before anything renders them', () => {
    expect(
      enrollmentInviteFrom({ username: 'bob', full_name: 'Bob Lee', add_device: false })
    ).toEqual(
      expect.objectContaining({ username: 'bob', full_name: 'Bob Lee', add_device: false })
    );
    expect(() => enrollmentInviteFrom({ full_name: 'Bob Lee' })).toThrow();
    expect(legacyTokenFrom({ invitation: 'tok' })).toBe('tok');
    expect(() => legacyTokenFrom({})).toThrow();
  });
});

describe('workspace words', () => {
  it('names the workspace by its S2 name, else the saved connection', () => {
    expect(workspaceLabelFor([connection], connection.id, makeSnapshot())).toBe('lab');
    const unnamed = makeSnapshot();
    delete unnamed.workspace.name;
    expect(workspaceLabelFor([connection], connection.id, unnamed)).toBe('Fixture');
  });

  it('asks a typed confirmation for the bare name, never the name — server label', () => {
    const unnamed = makeSnapshot();
    delete unnamed.workspace.name;
    const twin = { ...connection, id: 'conn-2', ssh_target: 'alice@other.example.edu' };
    expect(workspaceLabelFor([connection, twin], connection.id, unnamed)).toBe(
      'Fixture — hpc.example.edu'
    );
    expect(workspacePhraseFor([connection, twin], connection.id, unnamed)).toBe('Fixture');
    expect(workspacePhraseFor([connection, twin], connection.id, makeSnapshot())).toBe('lab');
  });

  it('reads S2 support from the projected handles', () => {
    expect(uniqueNamesSupported(makeSnapshot())).toBe(false);
    const s2 = makeSnapshot();
    s2.channels[0].handle = 'general';
    expect(uniqueNamesSupported(s2)).toBe(true);
    expect(uniqueNamesSupported(null)).toBe(false);
  });

  it('computes the fingerprint the Join dialog shows', async () => {
    const fingerprint = await workspaceKeyFingerprint('ab'.repeat(32));
    expect(fingerprint).toBe('9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885');
    expect(groupedFingerprint(fingerprint!)).toBe('9A2D B2E2 3F15 04CD');
    expect(await workspaceKeyFingerprint('not-a-key')).toBeNull();
  });
});
