import type { ReasoningEffort } from '../api/types.gen';

export type { ReasoningEffort };

const STORAGE_PREFIX = 'biorouter.reasoningEffort.v2:';

export const REASONING_EFFORTS: ReasoningEffort[] = ['quick', 'normal', 'deep'];
export const DEFAULT_REASONING_EFFORT: ReasoningEffort = 'normal';

export const REASONING_EFFORT_LABELS: Record<ReasoningEffort, string> = {
  quick: 'Quick',
  normal: 'Normal',
  deep: 'Deep',
};

export const REASONING_EFFORT_DESCRIPTIONS: Record<ReasoningEffort, string> = {
  quick: 'Fast answers with minimal exploration.',
  normal: "Use this chat's /effort setting, or the model's default depth.",
  deep: 'More thinking and exploration.',
};

export function sessionReasoningScope(sessionId: string): string {
  return `session:${sessionId}`;
}

export function draftReasoningScope(draftKey: string): string {
  return `draft:${draftKey}`;
}

function isReasoningEffort(value: unknown): value is ReasoningEffort {
  return typeof value === 'string' && (REASONING_EFFORTS as string[]).includes(value);
}

const current = new Map<string, ReasoningEffort>();
const listeners = new Map<string, Set<() => void>>();

function storageForScope(scope: string): Storage {
  // Home and unsent tabs belong to their window; established chats travel with
  // their session id when resumed or opened in another window.
  return scope.startsWith('session:') ? localStorage : sessionStorage;
}

export function getReasoningEffort(scope: string): ReasoningEffort {
  if (!current.has(scope)) {
    let effort = DEFAULT_REASONING_EFFORT;
    try {
      const stored = storageForScope(scope).getItem(STORAGE_PREFIX + scope);
      if (isReasoningEffort(stored)) effort = stored;
    } catch {
      // The in-memory choice still works when storage is unavailable.
    }
    current.set(scope, effort);
  }
  return current.get(scope)!;
}

export function setReasoningEffort(scope: string, effort: ReasoningEffort): void {
  if (getReasoningEffort(scope) === effort) return;
  current.set(scope, effort);
  try {
    storageForScope(scope).setItem(STORAGE_PREFIX + scope, effort);
  } catch {
    // The in-memory choice still works when storage is unavailable.
  }
  listeners.get(scope)?.forEach((listener) => listener());
}

export function subscribeToReasoningEffort(scope: string, listener: () => void): () => void {
  let scopedListeners = listeners.get(scope);
  if (!scopedListeners) listeners.set(scope, (scopedListeners = new Set()));
  scopedListeners.add(listener);
  return () => {
    scopedListeners.delete(listener);
    if (scopedListeners.size === 0) listeners.delete(scope);
  };
}

window.addEventListener('storage', (event) => {
  if (event.key !== null && !event.key.startsWith(STORAGE_PREFIX)) return;
  const scopes = event.key ? [event.key.slice(STORAGE_PREFIX.length)] : [...current.keys()];
  for (const scope of scopes) {
    try {
      if (event.storageArea && event.storageArea !== storageForScope(scope)) continue;
    } catch {
      continue;
    }
    const previous = current.get(scope);
    current.delete(scope);
    if (getReasoningEffort(scope) !== previous) {
      listeners.get(scope)?.forEach((listener) => listener());
    }
  }
});

/** Normal yields to the session's /effort setting, as it did before scoping. */
export function reasoningEffortForRequest(effort: ReasoningEffort): ReasoningEffort | undefined {
  return effort === DEFAULT_REASONING_EFFORT ? undefined : effort;
}

/** Transfer before navigation so the new controller's first reply sees the choice. */
export function adoptDraftReasoningEffort(
  draftScope: string,
  sessionId: string,
  submittedEffort: ReasoningEffort
): void {
  setReasoningEffort(sessionReasoningScope(sessionId), submittedEffort);
  // A newer draft choice made while session creation awaited belongs to the
  // next message; do not clear it along with the submitted draft.
  if (getReasoningEffort(draftScope) === submittedEffort) {
    setReasoningEffort(draftScope, DEFAULT_REASONING_EFFORT);
  }
}

export function resetReasoningEffortForTests(): void {
  current.clear();
  listeners.clear();
}
