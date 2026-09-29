import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { ackPrivacyDisclosure, getPrivacyDisclosure } from '../../api';
import type { ProviderTier } from '../../api';
import { userActionHeaders } from '../../utils/userAction';

/**
 * The non-private-model disclosure, as the renderer sees it (issue #56, DR-17
 * requirement 3).
 *
 * ⚠ **The renderer holds no copy of its own.** The one definition lives in
 * `crates/biorouter/src/privacy/disclosure.rs` and arrives over
 * `GET /privacy/disclosure`. Four hand-written copies of a sentence drift within
 * one release and the drifted one is always the one a user reads — so every
 * surface in this app (the dialog, the settings panel, the provider grid, the
 * model chip) renders what this module was handed, and a hardcoded English
 * string in any of them is the defect Step 5's gate greps for.
 *
 * ⚠ **Nothing here reads the master privacy switch.** DR-15 turns off gates, the
 * ratchet and refusals; it does not turn off the truth, and with enforcement off
 * the exposure is *larger*, not smaller. Every other privacy surface in this
 * folder reads the switch, which is exactly why wiring this one the same way is
 * the plausible mistake.
 */
export interface DisclosureCopy {
  /** The dialog heading, with `{provider}` still in it. */
  titleTemplate: string;
  /** The long form: the blocking dialog and the settings panel. */
  long: string;
  /** The one-line form: the model chip's tooltip, the provider grid. */
  short: string;
  /**
   * Settings > App > Privacy's heading and long form, written about the class
   * rather than "this model" (W2-PRV-14). `null` from a daemon that predates
   * them, where the panel falls back to the dialog's copy.
   */
  settingsTitle?: string | null;
  settings?: string | null;
}

export interface DisclosureState {
  /** `null` until the copy has been fetched. */
  copy: DisclosureCopy | null;
  /** `null` until the daemon has answered. */
  acknowledged: boolean | null;
  /**
   * Record the acknowledgement, carrying DR-16's proof of user. Resolves `true`
   * only when the daemon actually recorded it — see {@link acknowledgeError}.
   */
  acknowledge: () => Promise<boolean>;
  /**
   * Why the last acknowledgement was **not** recorded, or `null`.
   *
   * ⚠ Load-bearing. The daemon refuses this POST with 403 when it holds no
   * user-action key — `UserActionProof::NoKeyInstalled`, which `auth.rs`
   * documents as the state of `just run-server`, a hand-run `biorouterd agent`
   * and every headless deployment. The generated client's default is
   * `ThrowOnError = false`, so it *returns* that refusal rather than throwing
   * it: an unconditional `setAcknowledged(true)` after the `await` would close
   * the dialog as though it had worked, write nothing, and re-present the same
   * blocking modal on every launch with no symptom the user could act on. A
   * confirmation a user sees daily is a confirmation they stop reading, which
   * is the outcome this whole task exists to prevent.
   */
  acknowledgeError: string | null;
  /**
   * The user was told the acknowledgement could not be saved, and went on.
   *
   * ⚠ Shared, like everything else here, and for a reason the two fixes only
   * have together: nothing was written, so `acknowledged` stays false, so a pane
   * that let the user past would release the dialog to the next pane waiting for
   * it. On a keyless daemon with a four-way split that is four identical
   * un-satisfiable dialogs to click through. It is deliberately NOT persisted —
   * the daemon holds no record, so the next launch asks again, which is the
   * honest behaviour.
   */
  dismissedUnrecorded: boolean;
  /** Record that the user went past an unrecorded acknowledgement. */
  dismissUnrecorded: () => void;
}

/**
 * The renderer's framing for an acknowledgement that was not written.
 *
 * ⚠ Not product copy about *the disclosure* — that lives in Rust and arrives
 * over the wire, and a second definition of it here is what gate (1) greps for.
 * This is the operational sentence around the daemon's own refusal text, which
 * is appended to it verbatim.
 */
export const ACK_FAILED_PREFIX = 'This acknowledgement could not be saved.';

/** {@link ACK_FAILED_PREFIX} plus whatever the daemon said, when it said anything. */
function describeAckFailure(detail: unknown): string {
  const said =
    typeof detail === 'string'
      ? detail.trim()
      : detail instanceof Error
        ? detail.message.trim()
        : '';
  return said ? `${ACK_FAILED_PREFIX} ${said}` : ACK_FAILED_PREFIX;
}

/**
 * The heading for `providerDisplayName`, from the served template.
 *
 * The substitution lives here rather than in the Rust route so that one fetched
 * copy serves every provider a user switches between without a round trip.
 */
export function disclosureTitle(copy: DisclosureCopy, providerDisplayName: string): string {
  return copy.titleTemplate.replace('{provider}', providerDisplayName);
}

/**
 * Must the user be told what this provider can reach?
 *
 * The tier, and only the tier — mirroring `disclosure::required_for`. Task 5
 * owns the private set, so a fourth private provider must switch this off with
 * no edit here.
 *
 * ⚠ Called only with metadata that has actually RESOLVED. An absent `tier` on a
 * resolved entry is Public by the daemon's own polarity (`#[serde(default)]`
 * yields the fail-safe tier), so it discloses; "we have not looked yet" is
 * represented by having no metadata to pass, not by `undefined` here.
 */
export function disclosureRequiredForTier(tier: ProviderTier | null | undefined): boolean {
  return tier !== 'private';
}

/**
 * The disclosure is a property of the INSTALL, so it is held once for the app
 * rather than once per component (issue #56, DR-17 requirement 3).
 *
 * ⚠ **Module-scoped on purpose.** `ChatGroupsShell` mounts one `BaseChat` per
 * chat group and the split view caps at six, so per-hook state gave a two-pane
 * split two independent gates: two stacked un-dismissible modals, each wanting
 * its own click, and acknowledging one left the other's `acknowledged` at
 * `false`. The panes have to agree about a fact that is true of the machine.
 * {@link useSoleDisclosurePresenter} then decides which single gate shows it.
 *
 * Consequences worth knowing: the copy is fetched once for the whole app rather
 * than once per surface, and a surface that mounts later gets the answer the
 * first one already has, with no round trip.
 */
interface DisclosureStore {
  copy: DisclosureCopy | null;
  acknowledged: boolean | null;
  acknowledgeError: string | null;
  dismissedUnrecorded: boolean;
  /** The gate currently showing the blocking dialog, if any. */
  presenter: symbol | null;
}

let store: DisclosureStore = {
  copy: null,
  acknowledged: null,
  acknowledgeError: null,
  dismissedUnrecorded: false,
  presenter: null,
};
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const readStore = () => store;
function patch(next: Partial<DisclosureStore>) {
  store = { ...store, ...next };
  // A copy, because a listener may claim the presenter and so patch again.
  for (const listener of [...listeners]) listener();
}

/** In flight, so N surfaces mounting together make one request between them. */
let fetching: Promise<void> | null = null;

function ensureFetched() {
  if (fetching || store.copy) return;
  fetching = (async () => {
    try {
      const result = await getPrivacyDisclosure();
      const served = result?.data;
      if (!served) return;
      // Read structurally: the generated client describes the daemon this tree
      // was built against, and an older daemon serves neither settings field.
      const extra = served as { settings?: unknown; settings_title?: unknown };
      patch({
        copy: {
          titleTemplate: served.title_template,
          long: served.long,
          short: served.short,
          settingsTitle: typeof extra.settings_title === 'string' ? extra.settings_title : null,
          settings: typeof extra.settings === 'string' ? extra.settings : null,
        },
        acknowledged: served.acknowledged,
      });
    } catch {
      // Leave both null. See `useDisclosure`'s doc comment. `fetching` is
      // cleared below, so a surface mounting later tries again.
    } finally {
      fetching = null;
    }
  })();
}

/**
 * Whether the person may be taken past the disclosure right now, once the
 * daemon has answered: `true` when it is acknowledged (or they were told it
 * could not be saved and went on), `false` when it is still due, and `null`
 * when the daemon could not say.
 *
 * T3-SH-1. For a send that has to wait for the answer rather than render
 * whatever the store holds so far: a chat started on a model held for it before
 * it was sent is bound in the same click that sends its first message, so the
 * gates that key on a bound provider answer only after that message has gone.
 *
 * `null` is the same "renders nothing" answer every surface gives a failed
 * fetch (see {@link useDisclosure}); the caller decides what to do with it.
 */
export async function readDisclosureAcknowledgement(): Promise<boolean | null> {
  ensureFetched();
  if (fetching) await fetching;
  if (store.acknowledged === true || store.dismissedUnrecorded) return true;
  return store.acknowledged;
}

/**
 * Drop everything this module remembers.
 *
 * ⚠ **Tests only.** The store outlives a `cleanup()` because it is module state,
 * so without this one test's acknowledgement would decide the next one's
 * starting position — the kind of order dependence that makes a suite pass in
 * isolation and fail in a run.
 */
export function __resetDisclosureStoreForTests() {
  store = {
    copy: null,
    acknowledged: null,
    acknowledgeError: null,
    dismissedUnrecorded: false,
    presenter: null,
  };
  fetching = null;
  listeners.clear();
}

/**
 * Which single gate may show the blocking dialog.
 *
 * First claimant wins and holds it until it stops wanting it or unmounts; the
 * others wait. `wants` is the gate's own answer to "should this be up at all",
 * so a pane bound to a local model never joins the queue.
 */
export function useSoleDisclosurePresenter(wants: boolean): boolean {
  const tokenRef = useRef<symbol | null>(null);
  if (tokenRef.current === null) tokenRef.current = Symbol('disclosure-presenter');
  const token = tokenRef.current;
  const [isPresenter, setIsPresenter] = useState(false);

  useEffect(() => {
    if (!wants) {
      setIsPresenter(false);
      return;
    }
    const attempt = () => {
      if (store.presenter === null) patch({ presenter: token });
      setIsPresenter(store.presenter === token);
    };
    attempt();
    // Subscribed, so that when the holder unmounts — a pane closed with the
    // dialog still up — a waiting gate takes it over rather than the disclosure
    // disappearing until the next launch.
    const unsubscribe = subscribe(attempt);
    return () => {
      unsubscribe();
      if (store.presenter === token) patch({ presenter: null });
    };
  }, [wants, token]);

  return isPresenter;
}

/**
 * Fetch the copy and the acknowledgement state, once for the whole app.
 *
 * A failed fetch leaves `copy` null and `acknowledged` null, and every surface
 * renders nothing rather than inventing prose. That is the one honest answer:
 * a blocking dialog with no text in it discloses nothing and only takes the app
 * away from the user.
 */
export function useDisclosure(enabled: boolean = true): DisclosureState {
  const { copy, acknowledged, acknowledgeError, dismissedUnrecorded } = useSyncExternalStore(
    subscribe,
    readStore
  );

  useEffect(() => {
    if (enabled) ensureFetched();
  }, [enabled]);

  const acknowledge = useCallback(async () => {
    // DR-16's proof of user. Without it the daemon refuses with a 403, which is
    // the correct direction: a model must not be able to dismiss this on the
    // user's behalf.
    //
    // ⚠ The ANSWER is read, not just awaited. This client does not throw on a
    // non-2xx by default — it hands the refusal back in `error` — so the two
    // outcomes are indistinguishable to an `await` alone, and the failing one is
    // the one that silently never writes the record. The returned shape is
    // checked rather than `throwOnError` because the refusal body is plain text
    // with no typed error to parse, and because a transport failure must land in
    // the same place as a policy refusal; both leave the record unwritten.
    let refusal: unknown;
    try {
      const result = await ackPrivacyDisclosure({ headers: await userActionHeaders() });
      refusal = result?.error;
    } catch (thrown) {
      refusal = thrown;
    }
    if (refusal !== undefined && refusal !== null) {
      patch({ acknowledgeError: describeAckFailure(refusal) });
      return false;
    }
    patch({ acknowledgeError: null, acknowledged: true });
    return true;
  }, []);

  const dismissUnrecorded = useCallback(() => {
    patch({ dismissedUnrecorded: true });
  }, []);

  return {
    copy,
    acknowledged,
    acknowledge,
    acknowledgeError,
    dismissedUnrecorded,
    dismissUnrecorded,
  };
}
