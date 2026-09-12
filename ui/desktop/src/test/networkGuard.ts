/**
 * The suite makes no HTTP requests, and says so out loud when one is attempted.
 *
 * ## Why this exists
 *
 * jsdom inherits Node's real `fetch`, so an un-stubbed request in a renderer
 * test does not fail — it *leaves the machine*. What comes back then depends on
 * what the developer happens to be running, which makes the suite's result a
 * property of their laptop rather than of the code. The concrete case, measured
 * on 2026-09-11:
 *
 *   ProviderCatalog.test.tsx renders the Local tab, which mounts
 *   `OllamaInlineCard`, whose mount effect (`OllamaInlineCard.tsx:45`) calls
 *   `checkOllamaStatus()` -> `fetch('http://127.0.0.1:11434/api/tags')`
 *   (`utils/ollamaDetection.ts:36`).
 *
 * On CI nothing listens on 11434, the request is refused in about a
 * millisecond, `isRunning` is false and the `if` at `OllamaInlineCard.tsx:39`
 * is not taken. On a developer's machine with `ollama serve` running, the
 * request succeeds, a SECOND request goes out for `hasModel()`, and two more
 * state updates land — after the test body has finished. Two different code
 * paths through the component, selected by an ambient daemon. That is worse
 * than a flaky test: it is green on CI and misbehaves only where a human will
 * be trained to ignore it.
 *
 * The other ten measured requests all went to `http://localhost` **port 80**
 * (the generated client's test base URL), where nothing listens here — so they
 * behaved like CI *by luck*. A developer with anything bound to port 80 would
 * start feeding real responses to `/sessions` and `/config/extensions`.
 *
 * ## The mechanism
 *
 * One default `fetch` in `src/test/setup.ts`, for the whole suite, which:
 *
 *   1. **rejects** every request, so behaviour is identical to CI's
 *      "nothing is listening" regardless of what the developer is running, and
 *   2. **records** the attempt, so the guard can report it even when the caller
 *      swallows the rejection — and `ollamaDetection.ts` does exactly that
 *      (`catch` -> `{ isRunning: false }`), which is why rejecting alone would
 *      be silent.
 *
 * A spec that legitimately needs `fetch` assigns its own (six already do); that
 * replaces this one and records nothing, which is the point.
 *
 * ## The allowance table is debt, not permission
 *
 * `KNOWN_NETWORK_ATTEMPTS` is the census measured across all 446 spec files on
 * 2026-09-11 — the specs that already reach for the network. They are recorded
 * rather than fixed because making each one's request succeed with stub data
 * would change what its component sees, and therefore risks changing what it
 * asserts; leaving the request to fail preserves today's behaviour exactly. A
 * spec NOT in this table that attempts a request fails immediately, with the
 * URL in the message. Entries should leave this table, never arrive.
 *
 * The Ollama host is deliberately absent from every entry, and
 * `networkGuard.test.ts` asserts it can never be added: that host is the one a
 * developer is actually likely to be running.
 */

/** Prefix of the rejection every un-stubbed request gets. */
export const OFFLINE_FETCH_MESSAGE = 'Un-stubbed network request in a test:';

/**
 * Hosts that must never appear in `KNOWN_NETWORK_ATTEMPTS`, because something
 * plausibly listens there on a developer's machine. Asserted, not documented.
 */
export const NEVER_ALLOWED_HOSTS = ['127.0.0.1:11434', 'localhost:11434'] as const;

/**
 * The measured census. Key: spec path from `src/` onwards. Value: the URL
 * prefixes that spec is known to attempt.
 */
export const KNOWN_NETWORK_ATTEMPTS: Readonly<Record<string, readonly string[]>> = {
  'src/components/workflows/shared/__tests__/WorkflowFormFields.test.tsx': [
    'http://localhost/config/extensions',
  ],
  'src/components/chatGroups/keyboardResubmitGuard.test.tsx': ['http://localhost/sessions'],
  'src/components/chatGroups/ChatGroupsShell.tearOff.test.tsx': ['http://localhost/sessions'],
  'src/components/chatGroups/ChatGroupsShell.terminal.test.tsx': ['http://localhost/sessions'],
  'src/components/chatGroups/ChatGroupsShell.sessionName.test.tsx': ['http://localhost/sessions'],
  'src/components/chatGroups/duplicateSubmissionWiring.test.tsx': ['http://localhost/sessions'],
  'src/components/settings/SettingsView.test.tsx': ['http://localhost/privacy/disclosure'],
  // Note what is NOT here: `http://127.0.0.1:11434/api/tags`. This spec used to
  // reach the developer's own Ollama; it now mocks `utils/ollamaDetection`.
  'src/components/settings/providers/ProviderCatalog.test.tsx': [
    'http://localhost/llamacpp/status',
    'http://localhost/config/detectable-providers',
  ],
  'src/components/settings/models/subcomponents/SwitchModelModal.privacy.test.tsx': [
    'http://localhost/llamacpp/status',
  ],
};

let attempts: string[] = [];

/** Every URL recorded since the last drain, in order. */
export function recordedAttempts(): readonly string[] {
  return attempts;
}

/** Forget what has been recorded. Used by the guard's own tests. */
export function resetRecordedAttempts(): void {
  attempts = [];
}

/**
 * Normalise a spec path to the `src/…` key shape used by the table. Returns the
 * input unchanged when it holds no `src/` segment, so an unknown shape fails
 * closed rather than matching an entry by accident.
 */
export function specKey(testPath: string | undefined): string {
  if (!testPath) return '<unknown spec>';
  const normalised = testPath.replace(/\\/g, '/');
  const at = normalised.lastIndexOf('/src/');
  return at === -1 ? normalised : normalised.slice(at + 1);
}

/** The recorded attempts this spec has no allowance for. */
export function unexpectedAttempts(
  testPath: string | undefined,
  recorded: readonly string[]
): string[] {
  const allowed = KNOWN_NETWORK_ATTEMPTS[specKey(testPath)] ?? [];
  return [...new Set(recorded.filter((url) => !allowed.some((prefix) => url.startsWith(prefix))))];
}

/**
 * Install the offline `fetch`. Re-installing is harmless; a spec that assigns
 * its own afterwards simply wins.
 */
export function installOfflineFetch(): void {
  // Typed off `fetch` itself rather than by naming `RequestInfo`/`RequestInit`,
  // which tsc knows from lib.dom but eslint's `no-undef` does not.
  type FetchInput = Parameters<typeof fetch>[0];
  type FetchInit = Parameters<typeof fetch>[1];
  globalThis.fetch = ((input: FetchInput, init?: FetchInit) => {
    const asRequest = input as { url?: string; method?: string };
    const url =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.toString()
          : (asRequest?.url ?? String(input));
    attempts.push(url);
    const method = (init?.method ?? asRequest?.method ?? 'GET').toUpperCase();
    return Promise.reject(
      new Error(
        `${OFFLINE_FETCH_MESSAGE} ${method} ${url}\n` +
          'Tests must not reach the network — what comes back would depend on what the ' +
          'developer happens to be running. Mock the module that makes this call (see the ' +
          '`vi.mock` of utils/ollamaDetection in App.test.tsx for the shape), or assign ' +
          'your own `globalThis.fetch` in the spec. src/test/networkGuard.ts explains why.'
      )
    );
  }) as typeof fetch;
}

/**
 * Fail the current test if an un-allowed request was attempted, then drain.
 * Called from `setup.ts` after `cleanup()` — unmounting is what flushes the
 * passive effects that make these calls, so checking before it would miss them.
 */
export function assertNoUnexpectedNetworkAttempts(testPath: string | undefined): void {
  const recorded = attempts;
  attempts = [];
  const unexpected = unexpectedAttempts(testPath, recorded);
  if (unexpected.length === 0) return;
  throw new Error(
    `${OFFLINE_FETCH_MESSAGE}\n` +
      unexpected.map((url) => `  ${url}`).join('\n') +
      `\n\nThis spec (${specKey(testPath)}) reached for the network, so its result would ` +
      'depend on what is running on this machine. Mock the module that makes the call. ' +
      'See src/test/networkGuard.ts.'
  );
}
