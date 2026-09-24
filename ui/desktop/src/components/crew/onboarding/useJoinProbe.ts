import { useEffect } from 'react';
import { joinStatus as fetchJoinStatus } from '../api/join';
import { isStaleDaemon } from '../api/errors';
import { useCrew } from '../state/CrewControllerContext';
import { updateJoinContext, useJoinContext } from './joinContext';
import { isUnknownDeviceFailure } from './joinText';
import { LEGACY_JOIN_STATUS } from './JoinStatusCard';

/**
 * Notice that the selected connection is connected but this computer is not a member yet, and
 * report it to the controller, whose `screen` then turns to `join`. The layout mounts it once,
 * whatever the screen; the join card then keeps the status current while it is shown.
 *
 * It asks `GET …/join` only when there is reason to: the workspace refused this computer as an
 * unknown device, or this computer saved the connection from Join or Host and has not been
 * verified since. A member whose updates failed for some other reason is never sent to the join
 * screen. A workspace whose server cannot join by code, or a daemon too old to ask, is reported
 * as the invitation-token path — but only for a computer the workspace does not know.
 *
 * It also clears the remembered "joining" flag once the workspace verifies this computer.
 */
export function useJoinProbe(): void {
  const crew = useCrew();
  const {
    connectionId,
    connection,
    snapshot,
    observedPrivacy,
    refreshError,
    joinStatus,
    setJoinStatus,
  } = crew;
  const context = useJoinContext(connectionId);
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const unknownDevice = isUnknownDeviceFailure(refreshError);
  const expectingJoin = Boolean(context.joining || context.hostSetup);

  useEffect(() => {
    if (verified && context.joining) updateJoinContext(connectionId, { joining: false });
  }, [verified, context.joining, connectionId]);

  const shouldProbe = Boolean(
    connectionId &&
    connection?.status === 'connected' &&
    !verified &&
    joinStatus === null &&
    (unknownDevice || expectingJoin)
  );

  useEffect(() => {
    if (!shouldProbe) return;
    const controller = new AbortController();
    fetchJoinStatus(connectionId, controller.signal).then(
      (result) => {
        if (controller.signal.aborted) return;
        // Probed only for a computer the workspace does not know, so a server that cannot join
        // by code leaves the invitation-token path.
        setJoinStatus(result.status === 'unsupported' ? LEGACY_JOIN_STATUS : result.status);
      },
      (failure: unknown) => {
        if (controller.signal.aborted) return;
        // An older daemon has no join route: only the token path can help a computer the
        // workspace does not know. Any other failure leaves the status unknown; the connection
        // bar already shows why updates stopped.
        if (isStaleDaemon(failure)) setJoinStatus(LEGACY_JOIN_STATUS);
      }
    );
    return () => controller.abort();
  }, [shouldProbe, connectionId, setJoinStatus]);
}
