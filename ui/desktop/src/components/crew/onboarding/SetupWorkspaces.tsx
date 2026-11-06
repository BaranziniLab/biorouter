import type { ReactNode } from 'react';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { sanitizeDisplayText } from '../identity';
import { openCopy, setupCopy, trustCopy } from './copy';
import { hostKeyFingerprints } from './joinText';
import { setupWorkspaceKey, type SetupWorkspace } from './setup';

/**
 * A found workspace as a person reads it, host named (AO-6's trust rule): "baranzini-lab, hosted
 * by @wanjun", or "…, hosted by you" for the account's own.
 */
export function foundWorkspaceTitle(entry: SetupWorkspace): string {
  const name = sanitizeDisplayText(entry.name) || openCopy.unnamed;
  if (entry.role === 'host') return openCopy.hostedByYou(name);
  const host = sanitizeDisplayText(entry.hostUsername);
  return host ? openCopy.hostedBy(name, host) : name;
}

/**
 * The facts under a found workspace: what you are there, where it stands, its Crew. A workspace
 * you host already says so in its title ("…, hosted by you"), so the role is not said twice.
 */
export function foundWorkspaceMeta(entry: SetupWorkspace): string {
  return [
    entry.role === 'host' ? null : openCopy.role[entry.role],
    openCopy.state[entry.state],
    entry.version ? openCopy.version(entry.version) : null,
    openCopy.thisComputer[entry.thisComputer] || null,
  ]
    .filter(Boolean)
    .join(' · ');
}

/**
 * The workspaces a setup job found on the server (AO-6, AO-8), one row each: its name and host,
 * what the account is there and where it stands. `actions` adds a row's buttons (Open a workspace
 * on a server); without it the list only says what is there (the Host and Join dialogs).
 */
export function SetupWorkspaceList({
  workspaces,
  label,
  actions,
  note,
  testId = 'crew-setup-workspaces',
}: {
  workspaces: readonly SetupWorkspace[];
  label: string;
  actions?: (entry: SetupWorkspace) => ReactNode;
  /** Said under a row: why it can't be opened, a restart that failed, an update's warning. */
  note?: (entry: SetupWorkspace) => ReactNode;
  testId?: string;
}) {
  if (workspaces.length === 0) return null;
  return (
    <section className="flex min-w-0 flex-col gap-1.5" aria-label={label} data-testid={testId}>
      {/* Not the caps section style: the label holds the login the person typed, whose case
          is theirs (an SSH alias can be case-sensitive). */}
      <h3 className="text-label text-text-muted">{label}</h3>
      <ul className="biorouter-settings-list">
        {workspaces.map((entry) => {
          const buttons = actions?.(entry);
          const extra = note?.(entry);
          return (
            <li
              key={setupWorkspaceKey(entry)}
              className="biorouter-settings-row flex min-w-0 flex-col items-start gap-1 px-3 py-2"
              data-crew-found-workspace={entry.workspaceId}
              data-crew-found-row={setupWorkspaceKey(entry)}
            >
              <div className="flex w-full min-w-0 flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-col gap-0.5">
                  <span className="min-w-0 break-words text-body text-text-default">
                    {foundWorkspaceTitle(entry)}
                  </span>
                  <span className="text-supporting text-text-muted">
                    {foundWorkspaceMeta(entry)}
                  </span>
                </div>
                {buttons ? <div className="flex shrink-0 items-center gap-2">{buttons}</div> : null}
              </div>
              {entry.role === 'removed' ? (
                <p className="text-supporting text-text-muted">
                  {openCopy.removed(sanitizeDisplayText(entry.name) || openCopy.unnamed)}
                </p>
              ) : null}
              {entry.state === 'unknown' && entry.error ? (
                <p className="text-supporting text-text-muted">
                  {sanitizeDisplayText(entry.error)}
                </p>
              ) : null}
              {extra}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/**
 * A setup that stopped because the server's host key is unknown or changed: the fingerprint it
 * offered and how to verify it, as Join's trust pane says it (`TrustPane`). Crew never accepts a
 * host key; the person adds it, then tries again.
 */
export function SetupHostKeyHelp({
  code,
  detail,
  server,
}: {
  code: string;
  detail: string;
  server: string;
}) {
  const { offered, known } = hostKeyFingerprints(detail);
  if (code === 'crew_ssh_host_key_changed') {
    return (
      <div className="crew-onboard-stack" data-testid="crew-setup-host-key-changed">
        <p className="text-body text-text-default">{trustCopy.changedBody}</p>
        {known ? (
          <div className="crew-onboard-field">
            <span className="text-supporting text-text-muted">{trustCopy.previous}</span>
            <CopyField value={known} label={trustCopy.previous} truncate="middle" />
          </div>
        ) : null}
        {offered ? (
          <div className="crew-onboard-field">
            <span className="text-supporting text-text-muted">{trustCopy.newKey}</span>
            <CopyField value={offered} label={trustCopy.offeredLabel} truncate="middle" />
          </div>
        ) : null}
      </div>
    );
  }
  return (
    <div className="crew-onboard-stack" data-testid="crew-setup-host-key-unknown">
      <p className="text-body text-text-default">{trustCopy.unknownBody}</p>
      {offered ? (
        <div className="crew-onboard-field">
          <span className="text-supporting text-text-muted">{trustCopy.offered}</span>
          <CopyField value={offered} label={trustCopy.offeredLabel} truncate="middle" />
        </div>
      ) : null}
      <Disclosure label={trustCopy.howToVerify}>
        <ol className="crew-onboard-list text-body text-text-default">
          {trustCopy.steps(server).map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      </Disclosure>
    </div>
  );
}

/** The two SSH refusals a person answers by verifying the server's key outside Crew. */
export const HOST_KEY_CODES: readonly string[] = [
  'crew_ssh_host_key_unknown',
  'crew_ssh_host_key_changed',
];

/** The note a job leaves when the server's workspaces could not be listed. */
export function workspacesNoteText(note: string | null): string | null {
  const words = sanitizeDisplayText(note);
  return words ? setupCopy.workspacesNote(words) : null;
}
