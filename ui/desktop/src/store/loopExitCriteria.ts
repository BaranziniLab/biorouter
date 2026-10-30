import { setLoopExitCriteria } from '../api';
import type { ExitCriteria } from '../api';

const drafts = new Map<string, ExitCriteria | null>();
const prefix = 'biorouter.loopExitCriteria.v1:';

export function getDraftExitCriteria(scope: string): ExitCriteria | null {
  if (!drafts.has(scope)) {
    const stored = sessionStorage.getItem(prefix + scope);
    const value = stored ? JSON.parse(stored) : null;
    if (value !== null && (typeof value !== 'object' || Array.isArray(value))) {
      throw new Error('Saved loop exit criteria are invalid. Open Exit criteria to replace them.');
    }
    drafts.set(scope, value as ExitCriteria | null);
  }
  return drafts.get(scope) ?? null;
}

export function setDraftExitCriteria(scope: string, criteria: ExitCriteria | null): void {
  sessionStorage.setItem(prefix + scope, JSON.stringify(criteria));
  drafts.set(scope, criteria);
}

export async function adoptDraftExitCriteria(
  scope: string,
  sessionId: string,
  submitted: ExitCriteria | null
): Promise<void> {
  if (!submitted) return;
  await setLoopExitCriteria({
    path: { session_id: sessionId },
    body: { criteria: submitted },
    throwOnError: true,
  });
  if (getDraftExitCriteria(scope) === submitted) setDraftExitCriteria(scope, null);
}
