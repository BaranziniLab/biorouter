import { useEffect, useRef, useState } from 'react';
import { joinStatus as fetchJoinStatus } from '../api/join';
import { isStaleDaemon } from '../api/errors';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewJoinStatus } from '../state/types';
import { UNKNOWN_DEVICE_CODES } from '../state/observationFailure';
import { updateJoinContext, useJoinContext } from './joinContext';
import { isUnknownDeviceFailure } from './joinText';
import { LEGACY_JOIN_STATUS } from './JoinStatusCard';

/**
 * Why the probe asks `GET …/join`:
 * - `refused`: the workspace refused this computer as an unknown device — by the broker's code
 *   (`unauthorized`, `unknown_device`), or, from an older daemon, by its words;
 * - `expected`: this computer saved the connection from Join or Host and has not been verified;
 * - `unverified`: observation was refused (`observation_refused`) on a connected connection this
 *   window has never verified. That is also what a join started from a terminal looks like: it
 *   leaves no "joining" flag behind, and the daemon's frame names no device (T-14).
 */
type ProbeReason = 'refused' | 'expected' | 'unverified';

/** The answers that send no one anywhere: the join screen stays closed on either. */
const SETTLES_NOTHING: readonly CrewJoinStatus[] = ['joined', 'unsupported'];

/**
 * Notice that the selected connection is connected but this computer is not a member yet, and
 * report it to the controller, whose `screen` then turns to `join`. The layout mounts it once,
 * whatever the screen; the join card then keeps the status current while it is shown.
 *
 * It asks only when there is reason to (`ProbeReason`), and the daemon's answer decides: a member
 * whose updates failed for some other reason is answered `joined`, which never leads to the join
 * screen. A workspace whose server cannot join by code, or a daemon too old to ask, is reported as
 * the invitation-token path — but only for a computer the workspace refused or that is expecting a
 * join, never on the weak `unverified` evidence alone.
 *
 * It asks once per connection, except that an answer to the weak evidence which sent no one
 * anywhere (`joined`, `unsupported`) does not stop a stronger reason, arriving later, from asking
 * again: that answer was never about a computer the workspace refused.
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
  const refreshErrorCode = crew.refreshErrorCode ?? null;
  const context = useJoinContext(connectionId);
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  // The connections this window has seen verified. Kept here, where the probe lives for as long
  // as Crew is open, rather than in the controller.
  const everVerified = useRef(new Set<string>());
  useEffect(() => {
    if (verified && connectionId) everVerified.current.add(connectionId);
  }, [verified, connectionId]);

  useEffect(() => {
    if (verified && context.joining) updateJoinContext(connectionId, { joining: false });
  }, [verified, context.joining, connectionId]);

  const refused =
    (refreshErrorCode !== null && UNKNOWN_DEVICE_CODES.includes(refreshErrorCode)) ||
    isUnknownDeviceFailure(refreshError);
  const expected = Boolean(context.joining || context.hostSetup);
  const unverified =
    refreshErrorCode === 'observation_refused' && !everVerified.current.has(connectionId);
  const reason: ProbeReason | null = refused
    ? 'refused'
    : expected
      ? 'expected'
      : unverified
        ? 'unverified'
        : null;

  // What a weak probe recorded, while the controller still holds it for this connection.
  const [weakAnswer, setWeakAnswer] = useState<{
    connectionId: string;
    status: CrewJoinStatus;
  } | null>(null);
  const answeredOnlyWeakly =
    weakAnswer?.connectionId === connectionId && weakAnswer.status === joinStatus;
  const unanswered =
    joinStatus === null || (answeredOnlyWeakly && reason !== null && reason !== 'unverified');

  const probe: ProbeReason | null =
    connectionId && connection?.status === 'connected' && !verified && unanswered ? reason : null;

  useEffect(() => {
    if (!probe) return;
    const controller = new AbortController();
    const strong = probe !== 'unverified';
    fetchJoinStatus(connectionId, controller.signal).then(
      (result) => {
        if (controller.signal.aborted) return;
        // A server that cannot join by code leaves only the invitation-token path — for a
        // computer the workspace refused. On weaker evidence, record that the server was asked,
        // so the same evidence does not ask again, and send no one anywhere.
        const status: CrewJoinStatus =
          result.status === 'unsupported' && strong ? LEGACY_JOIN_STATUS : result.status;
        setWeakAnswer(
          !strong && SETTLES_NOTHING.includes(status) ? { connectionId, status } : null
        );
        setJoinStatus(status);
      },
      (failure: unknown) => {
        if (controller.signal.aborted) return;
        // An older daemon has no join route: only the token path can help a computer the
        // workspace refused. Any other failure leaves the status unknown; the connection bar
        // already shows why updates stopped.
        if (strong && isStaleDaemon(failure)) {
          setWeakAnswer(null);
          setJoinStatus(LEGACY_JOIN_STATUS);
        }
      }
    );
    return () => controller.abort();
  }, [probe, connectionId, setJoinStatus]);
}
