import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { inviteCopy, letInCopy, nameRuleCopy, refusalCopy } from './copy';
import {
  approveRefusalText,
  canonicalHint,
  directAddRefusalText,
  inviteRefusal,
  isAlreadyApproved,
  isNameRefusal,
  nameRefusalText,
  parseRefusal,
  refusalText,
} from './refusals';

/**
 * Every fixture is the broker's literal text (`crates/biorouter-crew/src/broker.rs`,
 * `broker/join.rs`, `invitation.rs`'s `DeviceCodeError`), with the names a real refusal would
 * carry. A paraphrase here would test a daemon that does not exist.
 */

/** How a daemon from before `broker_code` forwarded the same refusal (`crew/transport.rs`). */
function legacy(text: string): string {
  const code = /^([a-z_]+):/.exec(text)?.[1] ?? 'request_denied';
  return `Crew broker refused request: ${JSON.stringify({ code, message: text })}`;
}

const SENTENCES: [code: string, broker: string, shown: string][] = [
  [
    'name_taken (team)',
    'name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.',
    nameRuleCopy.teamTaken,
  ],
  [
    'name_taken (workspace)',
    'name_taken: Another workspace you host on this server is already using this name. Choose a different name.',
    'Another workspace you host on this server is already using this name. Choose a different name.',
  ],
  [
    'name_invalid',
    "name_invalid: Type the account's name on the server, not its numeric user ID.",
    "Type the account's name on the server, not its numeric user ID.",
  ],
  [
    'device_code_invalid',
    'device_code_invalid: Device codes never contain the letter U. Check the code.',
    'Device codes never contain the letter U. Check the code.',
  ],
  [
    'rate_limited (names)',
    'rate_limited: Too many name attempts. Try again later.',
    'Too many name attempts. Try again later.',
  ],
  [
    'rate_limited (challenges)',
    'rate_limited: too many live challenges',
    refusalCopy.tooManyAttempts,
  ],
  [
    'target_mismatch',
    'target_mismatch: The person you chose no longer has that username. Refresh and choose again.',
    'The person you chose no longer has that username. Refresh and choose again.',
  ],
  [
    'not_invited (joiner)',
    'not_invited: This account has no invitation to join this workspace. Ask the host to invite you.',
    'This account has no invitation to join this workspace. Ask the host to invite you.',
  ],
  [
    'not_invited (host)',
    'not_invited: @eve has no pending invitation. Invite them first.',
    '@eve has no pending invitation. Invite them first.',
  ],
  [
    'identity_unavailable (lookup)',
    "identity_unavailable: @Bob can't be matched to one account on this server.",
    "@Bob can't be matched to one account on this server.",
  ],
  [
    'identity_unavailable (name)',
    "identity_unavailable: This account's name on the server can't be used to join by invitation. Invite it with an enrollment token instead.",
    "This account's name on the server can't be used to join by invitation. Invite it with an enrollment token instead.",
  ],
  [
    'unknown_account',
    'unknown_account: There is no account @zed on this server. Check the spelling.',
    'There is no account @zed on this server. Check the spelling.',
  ],
  [
    'quota_exceeded (join quota)',
    'quota_exceeded: 100 people are already waiting to join. Cancel an invitation or wait for one to expire.',
    '100 people are already waiting to join. Cancel an invitation or wait for one to expire.',
  ],
  [
    // `open_inner` only: the broker says this when it starts, never in answer to a request.
    'quota_exceeded (journal, at startup)',
    'quota_exceeded: journal exceeds supported replay size of 1 GiB',
    refusalCopy.storageFull,
  ],
  [
    'quota_exceeded (journal)',
    'quota_exceeded: retained audit journal exceeds 1 GiB; preserve the complete store and use a new workspace; in-place audit deletion is not supported',
    refusalCopy.storageFull,
  ],
  [
    'quota_exceeded (state)',
    'quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported',
    refusalCopy.storageFull,
  ],
  [
    'quota_exceeded (operations)',
    'quota_exceeded: workspace operation quota requires maintenance',
    refusalCopy.storageFull,
  ],
  [
    'identity_conflict (member)',
    'identity_conflict: another active member is @bob; remove the old @bob first',
    refusalCopy.identityConflict('bob'),
  ],
  [
    'identity_conflict (dotted member)',
    'identity_conflict: another active member is @j.doe; remove the old @j.doe first',
    'Another active member is already @j.doe. Remove the old @j.doe first.',
  ],
  [
    'identity_conflict (invited)',
    'identity_conflict: Another account on this server is already invited as @bob. Cancel that invitation first.',
    'Another account on this server is already invited as @bob. Cancel that invitation first.',
  ],
  [
    'device_conflict',
    'device_conflict: this device key is already enrolled in this workspace; use a new device key',
    refusalCopy.deviceConflict,
  ],
  [
    'identity_mismatch (enrollment)',
    'identity_mismatch: enrollment principal changed; request a new invitation explicitly identifying the existing principal or offboard the old account',
    refusalCopy.identityMismatch,
  ],
  [
    'identity_mismatch (invitation)',
    'identity_mismatch: UID account name changed; offboard the old principal before enrollment',
    refusalCopy.identityMismatch,
  ],
  [
    'identity_mismatch (invite)',
    'identity_mismatch: This account joined as @bob and is now @robert on the server. Remove @bob first.',
    'This account joined as @bob and is now @robert on the server. Remove @bob first.',
  ],
  [
    'identity_ambiguous',
    'identity_ambiguous: This server spells the account @alice. Invite @alice.',
    'This server spells the account @alice. Invite @alice.',
  ],
  [
    'already_member',
    'already_member: @bob is already a member. Choose Add device to add another computer for them.',
    '@bob is already a member. Choose Add device to add another computer for them.',
  ],
  [
    'already_approved',
    'already_approved: You already let a device in for @eve. Replace the code only if they sent you a new one.',
    'You already let a device in for @eve. Replace the code only if they sent you a new one.',
  ],
  [
    'join_expired',
    'join_expired: This invitation expired. Ask the host to invite you again.',
    'This invitation expired. Ask the host to invite you again.',
  ],
  [
    'join_changed',
    'join_changed: The host sent a new invitation. Check your join status and try again.',
    'The host sent a new invitation. Check your join status and try again.',
  ],
  [
    'account_changed',
    'account_changed: This server account changed since the host invited it. Ask the host to invite you again.',
    'This server account changed since the host invited it. Ask the host to invite you again.',
  ],
  [
    'code_mismatch',
    "code_mismatch: The host hasn't let this device in. Send the host the code shown on your screen.",
    "The host hasn't let this device in. Send the host the code shown on your screen.",
  ],
  [
    'forbidden (not reworded)',
    'forbidden: only a person can invite or admit people',
    'forbidden: only a person can invite or admit people',
  ],
  [
    'invalid_params (not reworded)',
    'invalid_params: invite by username, or by uid and public_key, not both',
    'invalid_params: invite by username, or by uid and public_key, not both',
  ],
];

describe('refusalText', () => {
  it.each(SENTENCES)('words %s from the daemon’s own text', (_code, broker, shown) => {
    expect(refusalText(broker)).toBe(shown);
  });

  it.each(SENTENCES)(
    'words %s the same from an older daemon’s envelope',
    (_code, broker, shown) => {
      expect(refusalText(legacy(broker))).toBe(shown);
    }
  );

  it('never shows the envelope or its JSON', () => {
    for (const [, broker] of SENTENCES) {
      const text = refusalText(legacy(broker));
      expect(text).not.toContain('Crew broker refused request');
      expect(text).not.toContain('{');
    }
  });

  it('keeps an envelope that carries only a code searchable, and anything unreadable verbatim', () => {
    expect(refusalText('Crew broker refused request: {"code":"stale_cursor"}')).toBe(
      'stale_cursor'
    );
    expect(parseRefusal('Crew broker refused request: {"code":"stale_cursor"}').code).toBe(
      'stale_cursor'
    );
    expect(refusalText('Crew broker refused request: not json')).toBe(
      'Crew broker refused request: not json'
    );
    expect(refusalText('Crew broker refused request: {"message":"no code"}')).toBe(
      'Crew broker refused request: {"message":"no code"}'
    );
    expect(refusalText('')).toBe('Crew couldn’t complete that action.');
  });

  it('shows a person-written code’s text verbatim when it does not read as a sentence', () => {
    expect(refusalText('not_invited: internal lookup failed')).toBe(
      'not_invited: internal lookup failed'
    );
  });
});

describe('storage-full fixtures', () => {
  /**
   * A reworded text that the broker never writes makes a check that can never fire, and a table
   * row for it passes all the same. The storage-full rows are therefore read back against the
   * broker's source, where each must appear exactly as written.
   */
  const broker = readFileSync(
    resolve(__dirname, '../../../../../../crates/biorouter-crew/src/broker.rs'),
    'utf8'
  );
  const storageFull = SENTENCES.filter(([, , shown]) => shown === refusalCopy.storageFull);

  it('covers the journal, state-size and operation quotas', () => {
    expect(storageFull.map(([label]) => label)).toEqual([
      'quota_exceeded (journal, at startup)',
      'quota_exceeded (journal)',
      'quota_exceeded (state)',
      'quota_exceeded (operations)',
    ]);
  });

  it.each(storageFull)('%s is the broker’s literal text', (_label, text) => {
    expect(broker).toContain(`"${text}"`);
  });
});

describe('parseRefusal', () => {
  it('reads the code from the daemon’s text and from an older daemon’s envelope alike', () => {
    const broker =
      'name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.';
    const current = parseRefusal(broker);
    const older = parseRefusal(legacy(broker));
    expect(current).toEqual({
      code: 'name_taken',
      sentence: nameRuleCopy.teamTaken,
      text: broker,
      raw: broker,
    });
    expect(older).toEqual({ ...current, raw: legacy(broker) });
  });
});

describe('names', () => {
  const team =
    'name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.';
  const channel =
    'name_taken: A channel with this name, or one that looks like it, already exists in this team. Choose a different name.';
  const workspace =
    'name_taken: Another workspace you host on this server is already using this name. Choose a different name.';

  it('puts a taken name on the name field in the S2 words, from either daemon', () => {
    for (const text of [team, legacy(team)]) {
      expect(isNameRefusal(text)).toBe(true);
      expect(nameRefusalText(text, 'team')).toBe(nameRuleCopy.teamTaken);
    }
    for (const text of [channel, legacy(channel)]) {
      expect(nameRefusalText(text, 'channel')).toBe(nameRuleCopy.channelTaken);
    }
    expect(nameRefusalText(workspace, 'workspace')).toBe(
      'Another workspace you host on this server is already using this name. Choose a different name.'
    );
  });

  it('keeps a rate limit off the name field', () => {
    const limited = 'rate_limited: Too many name attempts. Try again later.';
    expect(isNameRefusal(limited)).toBe(false);
    expect(refusalText(legacy(limited))).toBe('Too many name attempts. Try again later.');
  });
});

describe('canonical spelling', () => {
  it.each([
    ['identity_ambiguous: This server spells the account @alice. Invite @alice.', 'alice'],
    ['identity_ambiguous: @Al is an alias on this server. Invite @alice.', 'alice'],
    ['identity_ambiguous: This server spells the account @j.doe. Invite @j.doe.', 'j.doe'],
    ['identity_ambiguous: @jd is an alias on this server. Invite @j.doe_2-x.', 'j.doe_2-x'],
  ])('reads the account %j names as %j, without the full stop', (text, canonical) => {
    expect(canonicalHint(parseRefusal(text).sentence)).toBe(canonical);
    expect(canonicalHint(parseRefusal(legacy(text)).sentence)).toBe(canonical);
  });

  it('offers the exact spelling back instead of the typed one', () => {
    expect(
      inviteRefusal(
        'identity_ambiguous: This server spells the account @alice. Invite @alice.',
        'Alice',
        'lab'
      )
    ).toEqual({ text: inviteCopy.refusal.canonical('alice'), alreadyMember: false });
    expect(
      inviteRefusal(
        legacy('identity_ambiguous: @Al is an alias on this server. Invite @alice.'),
        'Al',
        'lab'
      ).text
    ).toBe(inviteCopy.refusal.canonical('alice'));
    expect(
      inviteRefusal(
        'identity_ambiguous: This server spells the account @j.doe. Invite @j.doe.',
        'J.Doe',
        'lab'
      ).text
    ).toBe(inviteCopy.refusal.canonical('j.doe'));
  });
});

describe('invite and approve', () => {
  it('reads already_member and unknown_account from an older daemon too', () => {
    const member =
      'already_member: @bob is already a member. Choose Add device to add another computer for them.';
    expect(inviteRefusal(legacy(member), 'bob', 'lab')).toEqual({
      text: inviteCopy.refusal.alreadyMember('bob', 'lab'),
      alreadyMember: true,
    });
    expect(
      inviteRefusal(
        legacy('unknown_account: There is no account @zed on this server. Check the spelling.'),
        'zed',
        'lab'
      )
    ).toEqual({ text: inviteCopy.refusal.noAccount('zed'), alreadyMember: false });
  });

  it('says a device was already let in, and never matches another code', () => {
    const approved =
      'already_approved: You already let a device in for @eve. Replace the code only if they sent you a new one.';
    for (const text of [approved, legacy(approved)]) {
      expect(isAlreadyApproved(text)).toBe(true);
      expect(approveRefusalText(text, 'eve')).toBe(letInCopy.alreadyApproved('eve'));
    }
    expect(letInCopy.alreadyApproved('eve')).toBe('You already let a device in for @eve.');
    expect(
      isAlreadyApproved('not_invited: @eve has no pending invitation. Invite them first.')
    ).toBe(false);
    expect(
      approveRefusalText('not_invited: @eve has no pending invitation. Invite them first.', 'eve')
    ).toBe('@eve has no pending invitation. Invite them first.');
  });
});

describe('direct add', () => {
  // Literal texts from `broker.rs` (`mutate_team_add_member`, `mutate_channel_add_member`,
  // `DIRECT_ADD_UNKNOWN_CHANNEL`, `TARGET_MISMATCH`).
  it('drops the code from the broker’s person-written direct-add refusals', () => {
    expect(
      directAddRefusalText(
        'forbidden: You can only add people to channels you own. Uncheck #methods and try again.'
      )
    ).toBe('You can only add people to channels you own. Uncheck #methods and try again.');
    expect(
      directAddRefusalText(
        "invalid_params: One of the chosen channels isn't in this team. Refresh and choose again."
      )
    ).toBe("One of the chosen channels isn't in this team. Refresh and choose again.");
    expect(
      directAddRefusalText('channel_archived: #old is archived, so no one can be added to it.')
    ).toBe('#old is archived, so no one can be added to it.');
    expect(
      directAddRefusalText(
        'target_mismatch: The person you chose no longer has that username. Refresh and choose again.'
      )
    ).toBe('The person you chose no longer has that username. Refresh and choose again.');
  });

  it('keeps a technical text under the same codes verbatim, and leaves the shared list alone', () => {
    expect(directAddRefusalText('forbidden: team unavailable')).toBe('forbidden: team unavailable');
    // Outside a direct-add dialog the CLI-mirrored rule still shows the code.
    expect(
      refusalText("forbidden: Only the channel's owner or the workspace host can add people to it.")
    ).toBe("forbidden: Only the channel's owner or the workspace host can add people to it.");
  });
});
