import { Package } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { connectionServer, personFromProjection, usePeopleDirectory } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import { INSTALL_COMMANDS, notSetUpCopy } from './copy';
import { useJoinContext } from './joinContext';
import { firstName, sshUsername } from './joinText';
import { SetupCard, SetupScreen } from './parts';

/**
 * "Crew isn't set up for your account on {server}": `~/.local/bin/biorouter-crew` is missing for
 * this account (`crew_bridge_missing`), or sign-in worked but it did not start
 * (`crew_handoff_failed`). It is installed once per account, usually by the host or IT, so the
 * pane hands the person a message to send, keeps the commands behind "Install it yourself", and
 * offers Try again. It is the `connect` error's home while shown.
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

  return (
    <SetupScreen>
      <SetupCard icon={Package} title={notSetUpCopy.title(server)} testId="crew-not-set-up">
        <p className="text-body text-text-default">{notSetUpCopy.body}</p>
        <CopyField
          multiline
          label={notSetUpCopy.messageLabel}
          value={notSetUpCopy.message(hostFirst, username, server)}
        />
        <Disclosure label={notSetUpCopy.installYourself}>
          <div className="crew-onboard-stack">
            <p className="text-body text-text-default">{notSetUpCopy.installIntro}</p>
            <CopyField multiline label={notSetUpCopy.installLabel} value={INSTALL_COMMANDS} />
          </div>
        </Disclosure>
        <div className="crew-onboard-actions">
          <Button
            type="button"
            disabled={isPending('connect')}
            onClick={() => void connect({ userInitiated: true })}
          >
            {notSetUpCopy.tryAgain}
          </Button>
        </div>
      </SetupCard>
    </SetupScreen>
  );
}
