import { useEffect, useId, useRef } from 'react';
import { Package } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { connectionServer, personFromProjection, usePeopleDirectory } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import { notSetUpCopy, setupCopy } from './copy';
import { useJoinContext } from './joinContext';
import { firstName, isWorkspaceName, sshUsername } from './joinText';
import { SetupCard, SetupScreen } from './parts';
import { takeMemberSetupRequest } from './setup';
import { ManualInstall, SetupProgress, useSetupArtifact, useSetupJob } from './SetupProgress';

/**
 * "Crew isn't set up for your account on {server}": `~/.local/bin/biorouter-crew` is missing for
 * this account (`crew_bridge_missing`), or sign-in worked but it did not start
 * (`crew_handoff_failed`). The way on is "Set up Crew for my account" (OW-9): a member setup job
 * installs the broker this build carries in the person's own account and connects, and the screen
 * moves on by itself once it has. Join asks for it before the person sees this pane, so it starts
 * by itself then. The older ways stay, folded: a message for the host, and the manual install.
 * Try again connects without setting anything up. It is the `connect` error's home while shown.
 */
export function NotSetUpPane() {
  const crew = useCrew();
  const { connection, connectionId, connect, isPending, lastVerified, snapshot } = crew;
  useCrewErrorSlot('connect');
  const context = useJoinContext(connectionId);
  const directory = usePeopleDirectory(snapshot ?? lastVerified?.snapshot ?? null);
  const host =
    (context.hostUsername
      ? personFromProjection({
          username: context.hostUsername,
          display_name: context.hostDisplayName,
        })
      : null) ?? directory.host;
  const server = connectionServer(connection) || connection?.name || '';
  const username = sshUsername(connection?.ssh_target) ?? (context.username || null);
  const hostFirst = host && !(directory.me && host.id === directory.me.id) ? firstName(host) : null;
  const hintId = useId();
  const support = useSetupArtifact(true);

  // The job connected the account and verified the workspace server: connect here too, so the
  // controller learns of it and the screen moves on.
  const connectRef = useRef(connect);
  useEffect(() => {
    connectRef.current = connect;
  });
  const job = useSetupJob({
    server,
    onFinished: () => void connectRef.current({ userInitiated: true }),
  });
  const outdated = job.phase === 'outdated' || support.supported === false;

  // The work folder is named for the workspace (`~/crew-work/<workspace>`) when its name is one.
  const workspaceName = context.workspaceName ?? connection?.name ?? '';
  const setUp = () => {
    if (!connectionId || job.busy) return;
    void job.start({
      target: { kind: 'connection', connection_id: connectionId },
      intent: 'member',
      ...(isWorkspaceName(workspaceName) ? { workspace_name: workspaceName } : {}),
    });
  };

  // Join pressed for this connection a moment ago, and connecting found Crew missing: that press
  // was the consent, so setup starts by itself, once.
  const autoStarted = useRef(false);
  useEffect(() => {
    if (autoStarted.current || !connectionId) return;
    if (!takeMemberSetupRequest(connectionId)) return;
    autoStarted.current = true;
    setUp();
    // `setUp` is recreated every render; the request is taken once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId]);

  return (
    <SetupScreen>
      <SetupCard icon={Package} title={notSetUpCopy.title(server)} testId="crew-not-set-up">
        <p className="text-body text-text-default">{notSetUpCopy.body}</p>
        {outdated ? (
          <Note tone="warning" role="status">
            {setupCopy.outdated}
          </Note>
        ) : (
          <div className="crew-onboard-stack">
            <div className="crew-onboard-row">
              <Button
                type="button"
                // Not `disabled` while it runs: a disabled control drops keyboard focus to the
                // page (Q2-20). It says so, and does nothing.
                aria-disabled={job.busy || undefined}
                className="crew-onboard-waiting"
                aria-describedby={hintId}
                onClick={setUp}
              >
                {notSetUpCopy.setUp}
              </Button>
            </div>
            <p id={hintId} className="text-supporting text-text-muted">
              {notSetUpCopy.setUpHint(server, username)}
            </p>
            <SetupProgress job={job} intent="member" server={server} testId="crew-member-setup" />
          </div>
        )}
        <Disclosure label={notSetUpCopy.askHost}>
          <CopyField
            multiline
            label={notSetUpCopy.messageLabel}
            value={notSetUpCopy.message(hostFirst, username, server)}
          />
        </Disclosure>
        <Disclosure label={notSetUpCopy.installYourself}>
          <ManualInstall
            artifact={support.artifact}
            server={server}
            route={{
              ssh_target: connection?.ssh_target ?? '',
              port: connection?.port ?? null,
              proxy_jump: connection?.proxy_jump ?? null,
              identity_file: connection?.identity_file ?? null,
            }}
          />
        </Disclosure>
        <div className="crew-onboard-actions">
          <Button
            type="button"
            variant="secondary"
            disabled={isPending('connect') || job.busy}
            onClick={() => void connect({ userInitiated: true })}
          >
            {notSetUpCopy.tryAgain}
          </Button>
        </div>
      </SetupCard>
    </SetupScreen>
  );
}
