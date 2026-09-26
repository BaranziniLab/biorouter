import {
  connectionServer,
  displayNameIsUsername,
  sanitizeDisplayText,
  sanitizeUsername,
  type CrewPerson,
} from '../identity';

/**
 * Pure text helpers for the onboarding surfaces. None of them decides anything: the daemon parses
 * invitations and computes device codes, the broker admits people, and these only shape what a
 * person reads or copies.
 */

/** The login part of an SSH target (`bob` in `bob@hpc.example.edu`), or null for a bare alias. */
export function sshUsername(target: string | null | undefined): string | null {
  if (typeof target !== 'string') return null;
  const at = target.lastIndexOf('@');
  if (at <= 0) return null;
  return sanitizeUsername(target.slice(0, at)) || null;
}

/**
 * What to call a saved connection's server on screen (D-ALIAS): the daemon's `server_label` — the
 * person's own SSH alias for the address, when one maps to it — else the host of its SSH login.
 * Display only: the login itself (`ssh_target`) is what connects, and a value being written (a login
 * in Advanced, Connection settings' details) keeps the address.
 */
export function connectionServerLabel(
  connection: { ssh_target?: string | null; server_label?: unknown } | null | undefined
): string {
  const label =
    typeof connection?.server_label === 'string'
      ? sanitizeDisplayText(connection.server_label)
      : '';
  if (label) return label;
  return connection ? connectionServer({ id: '', ssh_target: connection.ssh_target ?? '' }) : '';
}

/**
 * The code the daemon records on a saved connection whose workspace ended this computer's
 * membership (`last_error_code` on `GET /crew/connections`, Q3-50).
 */
export const CREW_MEMBERSHIP_ENDED = 'crew_membership_ended';

/**
 * Whether the daemon recorded that the workspace ended this saved connection's membership. Read
 * defensively: the field is optional and additive, and a daemon that predates it says nothing.
 */
export function membershipEnded(connection: unknown): boolean {
  return (
    typeof connection === 'object' &&
    connection !== null &&
    (connection as { last_error_code?: unknown }).last_error_code === CREW_MEMBERSHIP_ENDED
  );
}

/**
 * How a person is addressed in a sentence that names them once more ("Send Alice this code"):
 * the first word of a chosen display name, else `@username`.
 */
export function firstName(person: CrewPerson | null | undefined): string | null {
  if (!person) return null;
  const username = sanitizeUsername(person.username);
  if (!username) return null;
  const name = person.displayName.trim();
  if (!name || displayNameIsUsername(name, username)) return `@${username}`;
  return name.split(/\s+/u)[0] || `@${username}`;
}

/**
 * A workspace key fingerprint as a person compares it by eye: the first 16 hex digits, upper-cased,
 * in groups of four (`3F2A 9C1E 77B0 D4E1`), the form `biorouter_crew::grouped_fingerprint`
 * prints. Null when the value has fewer than 16 hex digits.
 */
export function groupWorkspaceFingerprint(fingerprint: string | null | undefined): string | null {
  if (typeof fingerprint !== 'string') return null;
  const digits = fingerprint.replace(/[^0-9a-fA-F]/g, '').toUpperCase();
  if (digits.length < 16) return null;
  return (digits.slice(0, 16).match(/.{4}/g) ?? []).join(' ');
}

export interface HostKeyFingerprints {
  /** The key the server offered on this attempt. */
  offered: string | null;
  /** The key the known-hosts file holds for it, when OpenSSH named it. */
  known: string | null;
}

// The daemon's "Copy details" text starts with a summary of every host-key fingerprint OpenSSH
// printed, one per line (crates/biorouter/src/crew/transport.rs, `failure_detail`).
const FINGERPRINT_LINE =
  /^(New host key fingerprint \(offered by the server\)|Offered host key fingerprint|Previously known host key fingerprint|Host key fingerprint):\s*(SHA256:[A-Za-z0-9+/=]{32,})\s*$/;

/** The host-key fingerprints the daemon summarized at the top of a failure's detail. */
export function hostKeyFingerprints(detail: string | null | undefined): HostKeyFingerprints {
  const found: HostKeyFingerprints = { offered: null, known: null };
  if (typeof detail !== 'string') return found;
  let unlabeled: string | null = null;
  for (const line of detail.split('\n')) {
    const match = FINGERPRINT_LINE.exec(line.trim());
    if (!match) continue;
    const [, label, value] = match;
    if (label.startsWith('Previously known')) found.known ??= value;
    else if (label === 'Host key fingerprint') unlabeled ??= value;
    else found.offered ??= value;
  }
  found.offered ??= unlabeled;
  return found;
}

const WORKSPACE_NAME = /^[a-z0-9](?:[a-z0-9-]{0,38}[a-z0-9])?$/;
const UUID_SHAPED = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const HEX_64 = /^[0-9a-f]{64}$/;

/**
 * The workspace name a host's typing becomes, previewed live: lower case, runs of anything else
 * turned into one dash, at most 40 characters, starting and ending with a letter or digit
 * (naming design, "Workspace name"). The broker validates it again; this only previews it.
 */
export function workspaceSlug(text: string): string {
  const slug = text
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+/, '')
    .slice(0, 40)
    .replace(/-+$/, '');
  return slug;
}

/** Whether a name passes the S2 workspace-name rule (and is not shaped like an ID). */
export function isWorkspaceName(name: string): boolean {
  return WORKSPACE_NAME.test(name) && !UUID_SHAPED.test(name) && !HEX_64.test(name);
}

/**
 * The commands a host runs on the server to start Crew there with this computer's hosting key.
 * `--state-dir` and the `status` line keep them working with a `biorouter-crew` from before
 * workspace names; a newer one also takes `--name` and prints the invitation line from `start`.
 * The daemon reads whichever the paste holds.
 */
export function hostStartCommands(slug: string, bootstrapKey: string): string {
  const stateDir = `"$HOME/.local/share/biorouter-crew/${slug}"`;
  return [
    'umask 077',
    'mkdir -p "$HOME/.local/share/biorouter-crew"',
    `"$HOME/.local/bin/biorouter-crew" start --state-dir ${stateDir} --name ${slug} --bootstrap-key ${bootstrapKey}`,
    `"$HOME/.local/bin/biorouter-crew" status --state-dir ${stateDir}`,
  ].join('\n');
}

/**
 * What a host pasted from the terminal after running the start commands, made ready for the
 * daemon to read (T-27). The daemon still parses and validates everything; this only finds the
 * part it reads inside the terminal text around it, and names a problem it can see for itself.
 *
 * - `text`: hand this to the daemon. The `brcrew1:` line from `start`'s JSON, rejoined if the
 *   terminal copy broke it across lines; else the status JSON, on one line, without the prompt or
 *   other output around it; else the paste as it is, for the daemon to judge.
 * - `problem`: the paste shows why it can't hold the workspace yet — Crew was still starting, the
 *   copy stops partway through the JSON, `biorouter-crew` isn't installed, or it printed an error.
 */
export type StartOutput =
  | { kind: 'text'; text: string }
  | { kind: 'problem'; problem: 'starting' | 'cut-off' | 'not-installed' }
  | { kind: 'problem'; problem: 'server-error'; detail: string };

/** The invitation inside `start`'s JSON: everything up to the closing quote is the token. */
const INVITATION_IN_JSON = /"invitation"\s*:\s*"(brcrew1:[^"]*)"/;
const INVITATION_TOKEN = /brcrew1:/;
/** A shell that could not find `biorouter-crew` (bash, zsh and sh word it differently). */
const NOT_INSTALLED =
  /biorouter-crew[^\n]*(?:no such file or directory|command not found|not found)|(?:command not found|no such file or directory)[^\n]*biorouter-crew/i;
/** What `biorouter-crew` prints when a command fails (`Error: …` from its `main`). */
const CREW_ERROR = /^\s*Error:\s*(.+?)\s*$/m;
/** Only the start JSON's own keys mark an unfinished paste as Crew's; any other `{` is noise. */
const CREW_KEYS = /"(?:workspace_id|socket|started_pid|workspace_public_key|invitation)"/;
const MAX_ERROR_DETAIL = 200;

/** How many `{` the reader starts from before it stops looking: a paste is terminal output. */
const MAX_OBJECT_STARTS = 64;

/**
 * The `{…}` objects in the text, in order. One with no closing brace is reported unfinished
 * (`complete: false`), and the search goes on from the next `{`, so a stray brace in a prompt
 * cannot hide the JSON after it.
 */
function jsonObjects(text: string): { text: string; complete: boolean }[] {
  const found: { text: string; complete: boolean }[] = [];
  let start = text.indexOf('{');
  for (let starts = 0; start !== -1 && starts < MAX_OBJECT_STARTS; starts += 1) {
    let depth = 0;
    let inString = false;
    let escaped = false;
    let end = -1;
    for (let index = start; index < text.length; index += 1) {
      const char = text[index];
      if (inString) {
        if (escaped) escaped = false;
        else if (char === '\\') escaped = true;
        else if (char === '"') inString = false;
        continue;
      }
      if (char === '"') inString = true;
      else if (char === '{') depth += 1;
      else if (char === '}') {
        depth -= 1;
        if (depth === 0) {
          end = index;
          break;
        }
      }
    }
    if (end === -1) {
      found.push({ text: text.slice(start), complete: false });
      start = text.indexOf('{', start + 1);
    } else {
      found.push({ text: text.slice(start, end + 1), complete: true });
      start = text.indexOf('{', end + 1);
    }
  }
  return found;
}

function parseObject(text: string): Record<string, unknown> | null {
  try {
    // A terminal copy can break a long line anywhere, even inside a string; JSON has no raw
    // newlines of its own to lose.
    const value: unknown = JSON.parse(text.replace(/[\r\n]+/g, ''));
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

export function readStartOutput(pasted: string): StartOutput | null {
  if (!pasted.trim()) return null;

  const quoted = INVITATION_IN_JSON.exec(pasted);
  if (quoted) return { kind: 'text', text: quoted[1].replace(/\s+/g, '') };
  // A bare `brcrew1:` line: the daemon finds it wherever it sits.
  if (INVITATION_TOKEN.test(pasted)) return { kind: 'text', text: pasted };

  let starting = false;
  let invitationError: string | null = null;
  const objects = jsonObjects(pasted);
  for (const object of objects) {
    if (!object.complete) continue;
    const value = parseObject(object.text);
    if (!value) continue;
    if (typeof value.workspace_id === 'string' && typeof value.socket === 'string') {
      return { kind: 'text', text: object.text.replace(/[\r\n]+/g, '') };
    }
    if (value.state === 'starting' && 'started_pid' in value) starting = true;
    // `start` ran but could not write the invitation line; it says why.
    if (value.invitation === null && typeof value.invitation_error === 'string') {
      invitationError ??= value.invitation_error;
    }
  }
  // Crew's JSON that never closes, with nothing complete after it: the copy stopped early.
  const last = objects[objects.length - 1];
  if (last && !last.complete && CREW_KEYS.test(last.text)) {
    return { kind: 'problem', problem: 'cut-off' };
  }
  if (NOT_INSTALLED.test(pasted)) return { kind: 'problem', problem: 'not-installed' };
  if (starting) return { kind: 'problem', problem: 'starting' };
  const error = invitationError ?? CREW_ERROR.exec(pasted)?.[1] ?? null;
  if (error) {
    const detail = error.replace(/\s+/g, ' ').trim().slice(0, MAX_ERROR_DETAIL);
    if (detail) return { kind: 'problem', problem: 'server-error', detail };
  }
  return { kind: 'text', text: pasted };
}

/** The `ssh` command that signs in to the server as the connection will. */
export function sshLoginCommand(input: {
  ssh_target: string;
  port?: number | null;
  proxy_jump?: string | null;
  identity_file?: string | null;
}): string {
  const quote = (value: string) =>
    /^[A-Za-z0-9@%+=:,./_~-]+$/.test(value) ? value : `'${value.replace(/'/g, `'\\''`)}'`;
  const parts = ['ssh'];
  if (input.port && input.port !== 22) parts.push('-p', String(input.port));
  if (input.proxy_jump?.trim()) parts.push('-J', quote(input.proxy_jump.trim()));
  if (input.identity_file?.trim()) parts.push('-i', quote(input.identity_file.trim()));
  parts.push(quote(input.ssh_target.trim()));
  return parts.join(' ');
}

/**
 * Whether an observation failure says this computer's key is not a device of the workspace yet
 * (the broker's `unauthorized: unknown device`): the person connected but has not joined.
 */
export function isUnknownDeviceFailure(message: string | null | undefined): boolean {
  return typeof message === 'string' && /\bunknown device\b/i.test(message);
}

/**
 * A broker timestamp as a `Date`. The broker writes Unix seconds (`now()` in
 * `biorouter-crew/src/broker.rs`, documented on `CrewJoinStatus.expires_at`); a value already in
 * milliseconds (past the year 33658 as seconds) is taken as it is, so a future daemon that sends
 * milliseconds is not shown a date forty thousand years out. Null for anything not a finite,
 * positive number.
 */
export function brokerTime(value: number | null | undefined): Date | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return null;
  return new Date(value > 1e12 ? value : value * 1000);
}

/**
 * When an invitation stops working, in the words every Crew surface uses (Q4-36): "Sat 1:41 AM",
 * and whether that has already passed. Null when the status names no expiry.
 */
export function invitationExpiry(
  expiresAt: number | null | undefined,
  now: number = Date.now()
): { when: string; expired: boolean } | null {
  const date = brokerTime(expiresAt);
  if (!date) return null;
  const when = new Intl.DateTimeFormat(undefined, {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
  return { when, expired: date.getTime() <= now };
}

/**
 * A clock time to the second, for a line that must change on every attempt ("Tried again at
 * 9:41:07 AM", Q4-07): two attempts in one minute would otherwise read the same.
 */
export function attemptTime(at: number): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(at));
}
