import { useEffect, useState } from 'react';
import type { CrewConnectionView } from '../../../api';
import type { CrewSignInTarget } from '../CrewAuthentication';
import { crewHttp } from '../crewApi';
import { sanitizeDisplayText } from '../identity/displayText';
import { readSetup } from '../onboarding/setup';
import { serverLabel } from '../sidebar/sidebarView';

/**
 * The daemon's own name for what a sign-in card's window signs in to, named the way the Crew
 * view's own Sign in windows name it: a setup job by its SSH login (`ChatSetupJobs`,
 * `SetupProgress`), a saved connection by the person's name for its server (`SignInDialog`).
 *
 * Read from the daemon by the target's ID (`GET /crew/setup/{job_id}`, or the row with that ID in
 * `GET /crew/connections`), never taken from the card: the window connects to that ID, so its
 * title names what that ID is. Empty when the daemon names nothing.
 */
export async function readSignInWindowName(
  target: CrewSignInTarget,
  signal?: AbortSignal
): Promise<string> {
  if (target.kind === 'setup') {
    const run = await readSetup(target.jobId, signal);
    return sanitizeDisplayText(run.server?.login) || sanitizeDisplayText(run.server?.user);
  }
  const listed = await crewHttp<{ connections?: unknown }>(
    '/connections',
    'GET',
    undefined,
    signal
  );
  const rows: unknown[] = Array.isArray(listed?.connections) ? listed.connections : [];
  const connection = rows.find(
    (row): row is CrewConnectionView =>
      typeof row === 'object' && row !== null && (row as { id?: unknown }).id === target.id
  );
  return serverLabel(connection) || sanitizeDisplayText(connection?.name);
}

/**
 * {@link readSignInWindowName} for a card that can open its window (`enabled`), read once per
 * target. `null` until the daemon answers, and when it cannot.
 */
export function useSignInWindowName(target: CrewSignInTarget, enabled: boolean): string | null {
  const [name, setName] = useState<string | null>(null);
  const kind = target.kind;
  const targetId = target.kind === 'setup' ? target.jobId : target.id;
  useEffect(() => {
    setName(null);
    if (!enabled) return;
    const controller = new AbortController();
    const asked: CrewSignInTarget =
      kind === 'setup' ? { kind: 'setup', jobId: targetId } : { kind: 'connection', id: targetId };
    readSignInWindowName(asked, controller.signal).then(
      (read) => {
        if (!controller.signal.aborted) setName(read || null);
      },
      () => {
        // Quiet: the window then names "the server", and opening it says what went wrong.
      }
    );
    return () => controller.abort();
  }, [enabled, kind, targetId]);
  return name;
}
