import type { CrewEnrollmentInvite } from '../api/join';
import { isRecord, optionalText } from '../api/parse';
import { unexpectedCrewResponse } from '../api/errors';

/**
 * The legacy, token-based invitation (naming design, "What keeps verification material, and the
 * fallbacks"): the joiner's screen shows a join request — their username and this device's public
 * key — and the host turns it into a one-time token with the unchanged `enrollment.invite {uid,
 * public_key}`.
 *
 * The join request is free text a person pasted, so only its one load-bearing value is taken from
 * it: the 64-hex device key, and only when the text holds exactly one. The username in it, when
 * present, is used for help text (`id -u bob`) and never for authority — the host types the user
 * ID themselves, from the server.
 */
export interface JoinRequest {
  publicKey: string;
  username: string | null;
}

const HEX_KEY = /(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])/gi;
const USERNAME = /(?:^|[\s(])@([A-Za-z0-9._-]+)/;
const USERNAME_FIELD = /\busername\s*[:=]\s*@?([A-Za-z0-9._-]+)/i;

export function parseJoinRequest(text: string): JoinRequest | null {
  const keys = new Set((text.match(HEX_KEY) ?? []).map((key) => key.toLowerCase()));
  if (keys.size !== 1) return null;
  const [publicKey] = [...keys];
  const username = USERNAME_FIELD.exec(text)?.[1] ?? USERNAME.exec(text)?.[1] ?? null;
  return { publicKey, username };
}

/** The broker's answer to `enrollment.invite {username}`, checked before anything renders it. */
export function enrollmentInviteFrom(value: unknown): CrewEnrollmentInvite {
  if (!isRecord(value)) throw unexpectedCrewResponse('an invitation');
  const username = optionalText(value.username);
  if (!username) throw unexpectedCrewResponse('an invitation');
  return {
    username,
    full_name: optionalText(value.full_name) ?? null,
    add_device: value.add_device === true,
    join_id: typeof value.join_id === 'string' ? value.join_id : '',
    expires_at: typeof value.expires_at === 'number' ? value.expires_at : 0,
  };
}

/** The broker's answer to the legacy `enrollment.invite {uid, public_key}`: the one-time token. */
export function legacyTokenFrom(value: unknown): string {
  const token = isRecord(value) ? optionalText(value.invitation) : undefined;
  if (!token) throw unexpectedCrewResponse('an invitation token');
  return token;
}
