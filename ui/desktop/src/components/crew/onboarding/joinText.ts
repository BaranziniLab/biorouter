import { displayNameIsUsername, sanitizeUsername, type CrewPerson } from '../identity';

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
