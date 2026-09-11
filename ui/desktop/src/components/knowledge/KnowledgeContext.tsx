import {
  createContext,
  ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { listBases, getActive, setActive } from '../../api';
import { briefSelectionFailure } from './selectionWarning';
import { userActionHeaders } from '../../utils/userAction';
import { toastError } from '../../toasts';
/**
 * `KbListEntry` is `Manifest & { tier }` — the manifest the daemon stores plus
 * the privacy tier, which lives in `.kb-tiers` and not in `manifest.yaml`
 * (issue #56 DR-18). Every base this context hands out came from
 * `GET /knowledge/bases`, so every one of them knows its tier; a consumer that
 * only wants the manifest fields is unaffected, because the entry is a superset.
 */
import type { KbListEntry } from '../../api/types.gen';

const STORAGE_KEY_ACTIVE_KB = 'knowledge_active_kb';
const STORAGE_KEY_HIDDEN_KBS = 'knowledge_hidden_kbs';

function storageKeyForSession(sessionId: string | null | undefined): string {
  return sessionId ? `${STORAGE_KEY_ACTIVE_KB}:${sessionId}` : STORAGE_KEY_ACTIVE_KB;
}

function hiddenStorageKeyForSession(sessionId: string | null | undefined): string {
  return sessionId ? `${STORAGE_KEY_HIDDEN_KBS}:${sessionId}` : STORAGE_KEY_HIDDEN_KBS;
}

/**
 * What a selection change wants to happen to the primary pointer — the mirror
 * of the daemon's `PrimaryUpdate`. `unchanged` is what a set-only edit sends:
 * the daemon then re-establishes "the primary is a member of the set" itself
 * (promote to the first remaining base, or clear when none remain). Sending the
 * current primary back on a set-only edit would instead be *rejected* the
 * moment the user hides the primary, which is exactly when the repair matters.
 *
 * `inherit` is not a nicer spelling of `clear`. `clear` writes a *durable* "this
 * chat has no primary" that outranks the machine-wide default and survives every
 * other gesture, and deleting the base a chat had pinned installs exactly that
 * override in the chat. `inherit` drops the chat's own pointer so it follows the
 * machine-wide default again — the only way out of that state, and the reason
 * the daemon grew the third gesture at all.
 */
type PrimaryUpdate =
  | { kind: 'unchanged' }
  | { kind: 'clear' }
  | { kind: 'inherit' }
  | { kind: 'set'; id: string };

/** The shape both selection endpoints answer with — GET /active and POST /active. */
type SelectionPayload =
  | { primary_kb?: string | null; active_kb?: string | null; hidden_kbs?: string[] | null }
  | undefined;

/** `active_kb` is the deprecated mirror, read so a fresh renderer keeps working
 * against a daemon that predates `primary_kb`. */
function readPrimary(data: SelectionPayload): string | null {
  return data?.primary_kb ?? data?.active_kb ?? null;
}

/** `null` means "this answer did not state a set" (a daemon that predates the
 * field) — distinct from an empty set, and the caller must leave what it has
 * rather than erase the session's whole working set. */
function readHidden(data: SelectionPayload): string[] | null {
  return Array.isArray(data?.hidden_kbs)
    ? data.hidden_kbs.filter((id): id is string => typeof id === 'string')
    : null;
}

/**
 * Read one scope's selection — the chat's when `sessionId` is set, the
 * machine-wide default otherwise — as the person at the keyboard.
 *
 * ⚠ **Every selection read goes through here, and every one carries the
 * proof.** `GET /knowledge/active` naming a PRIVATE chat is on the reach gate's
 * list (`routes/session_reach.rs`) exactly as the POST in `syncSelection` is, and
 * the desktop gets through it the only way it can: `userActionHeaders()`, the
 * one helper that decides how this surface proves a person (issue #56 Task 58).
 * The reads used to go without it, so the daemon refused every private chat —
 * which is every chat on a UCSF install — and the Knowledge view, the chip and
 * the ingest target all showed this renderer's `localStorage` instead of the
 * daemon's selection (QA 2026-09-10 F14). It reaches nothing new: the same proof
 * already reads the chat's whole transcript through `getSession`.
 *
 * It sends the proof at machine scope too, where the gate is inert today, so
 * that a daemon which filters what an unproven caller may see never hands this
 * surface a selection with the user's own private bases missing from it.
 */
async function readSelection(sessionId: string | null) {
  const res = await getActive({
    query: sessionId ? { session_id: sessionId } : undefined,
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return res.data;
}

/**
 * The title of the one error a person sees when a selection change they made
 * did not land. A console line is not a report: the person clicked, the chip
 * moved, and without this nothing on screen would ever say it moved back.
 */
export const SELECTION_NOT_SAVED_TITLE = 'Knowledge base selection not saved';

interface KnowledgeContextType {
  bases: KbListEntry[];
  /** The session's knowledge bases — the one axis. Searchable, readable, usable. */
  visibleBases: KbListEntry[];
  loading: boolean;
  /**
   * Why `bases` cannot be trusted, when it cannot: the last list read failed and
   * nothing has replaced it. `loading` goes false either way, so without this a
   * consumer reads "the list is settled and this base is not in it" out of a
   * read that never happened — and acts on a base it has never seen. Non-empty
   * whenever a read failed (a rejection carrying no message is still a
   * failure), `null` once one lands.
   */
  basesError: string | null;
  /** The KB-less write target and the Knowledge view's subject. Always a member of visibleBases, or null. */
  primaryKb: KbListEntry | null;
  primaryKbId: string | null;
  hiddenKbIds: string[];
  /**
   * The machine-wide default primary — what a chat that has chosen nothing
   * shows. Read lazily by whoever offers the way back to it, since it is the
   * only thing that needs it; `null` until `refreshDefaultPrimary` has run, at
   * machine scope (where this chat-only affordance is irrelevant), and when it
   * names a base that is not installed.
   */
  defaultPrimaryKb: KbListEntry | null;
  /**
   * Whether offering "follow the default" would change anything a user can see:
   * this chat is holding its own pointer, the default names one of *this
   * chat's* bases, and it is not what the chat already shows.
   */
  canFollowDefaultPrimary: boolean;
  refreshDefaultPrimary: () => Promise<void>;
  /** Drop this chat's own primary so it follows the machine-wide default again. */
  followDefaultPrimary: () => void;
  setPrimaryKbId: (id: string | null) => void;
  setHiddenKbIds: (ids: string[]) => void;
  toggleKbHidden: (id: string) => void;
  hideAllKnowledgeBases: () => void;
  showAllKnowledgeBases: () => void;
  refresh: () => Promise<void>;
  /// Registered by KnowledgeGraphPanel so IngestPanel can request a re-fetch
  /// after each successful ingest. No-op if no graph is mounted.
  registerGraphRefresh: (fn: (() => Promise<void>) | null) => void;
  triggerGraphRefresh: () => void;
}

const KnowledgeContext = createContext<KnowledgeContextType | null>(null);

export function KnowledgeProvider({
  children,
  sessionId = null,
}: {
  children: ReactNode;
  sessionId?: string | null;
}) {
  const [bases, setBases] = useState<KbListEntry[]>([]);
  // Has a base list ever arrived? Until it has, `bases` being empty says nothing
  // about which bases exist, so no pointer may be judged missing against it.
  const [basesLoaded, setBasesLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [basesError, setBasesError] = useState<string | null>(null);
  const storageKey = useMemo(() => storageKeyForSession(sessionId), [sessionId]);
  const hiddenStorageKey = useMemo(() => hiddenStorageKeyForSession(sessionId), [sessionId]);
  // The pointer as last adopted — from the daemon, or optimistically from a
  // click. What consumers see is `primaryKbId` below, which also hides a
  // pointer at a base the list no longer holds.
  const [storedPrimaryKbId, setPrimaryKbIdState] = useState<string | null>(() =>
    localStorage.getItem(storageKeyForSession(sessionId))
  );
  const [hiddenKbIds, setHiddenKbIdsState] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem(hiddenStorageKeyForSession(sessionId));
      if (!raw) {
        return [];
      }
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed)
        ? parsed.filter((id): id is string => typeof id === 'string')
        : [];
    } catch {
      return [];
    }
  });
  // The machine-wide default, read on demand rather than on mount: only the
  // surface that offers the way back to it needs it, and hydrating it eagerly
  // would spend a request per chat switch on a value most chats never show.
  const [defaultPrimaryKbId, setDefaultPrimaryKbId] = useState<string | null>(null);
  const graphRefreshRef = useRef<(() => Promise<void>) | null>(null);
  // Every selection round-trip a USER starts — a write, its recovery read, the
  // hydrate on a chat switch — takes a generation. Only the newest may write
  // state back, so a slow answer cannot reinstate a selection the user has
  // already clicked past. A background re-read (`resyncSelection`) takes none of
  // its own; it may only land while the generation it started under is current.
  const selectionGenerationRef = useRef(0);
  // How many user writes are waiting on the daemon. A background re-read that
  // starts while one is out could be answered before the write commits, and
  // would then put the pre-write selection back over the write's answer.
  const writesInFlightRef = useRef(0);

  /**
   * Adopt a selection the DAEMON reported. `localStorage` is written here and
   * nowhere else: it holds the last selection the daemon confirmed, never a
   * guess, which is what lets a failed write fall back to it (QA 2026-09-10
   * F14 found it holding `soul` while the daemon had never stored it).
   */
  const applyPrimary = useCallback(
    (primary: string | null) => {
      setPrimaryKbIdState(primary);
      if (primary) localStorage.setItem(storageKey, primary);
      else localStorage.removeItem(storageKey);
    },
    [storageKey]
  );

  const applyHidden = useCallback(
    (hidden: string[]) => {
      setHiddenKbIdsState(hidden);
      localStorage.setItem(hiddenStorageKey, JSON.stringify(hidden));
    },
    [hiddenStorageKey]
  );

  /** Put back the last selection the daemon confirmed for this scope. */
  const restoreConfirmedSelection = useCallback(() => {
    setPrimaryKbIdState(localStorage.getItem(storageKey));
    try {
      const parsed: unknown = JSON.parse(localStorage.getItem(hiddenStorageKey) ?? '[]');
      setHiddenKbIdsState(
        Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : []
      );
    } catch {
      setHiddenKbIdsState([]);
    }
  }, [hiddenStorageKey, storageKey]);

  /**
   * Re-read this scope's selection and adopt it, WITHOUT superseding anything
   * the user is doing: skipped while a write is out, and dropped if a write or
   * a chat switch starts while the read is.
   *
   * This is how the renderer follows a selection that moved underneath it — a
   * base deleted (the daemon clears every pointer that named it), the agent's
   * `kb_set_active`, another window — and it replaces two effects that used to
   * "repair" this renderer's cache by WRITING: clearing a primary missing from
   * the base list, and pruning the hidden set against it. Both installed a
   * durable, session-scoped override the user never asked for, from whatever
   * list this renderer happened to hold, in every window at once. The daemon
   * already made the one repair the model allows (D2); the renderer reads it.
   */
  const resyncSelection = useCallback(async () => {
    if (writesInFlightRef.current > 0) return;
    const generation = selectionGenerationRef.current;
    try {
      const data = await readSelection(sessionId);
      if (generation !== selectionGenerationRef.current) return;
      applyPrimary(readPrimary(data));
      const hidden = readHidden(data);
      if (hidden) applyHidden(hidden);
    } catch (err) {
      // A failed read changes nothing on screen: it is not evidence the
      // selection moved, and the next refresh or hydrate asks again.
      console.warn('Knowledge selection not re-read:', briefSelectionFailure(err));
    }
  }, [applyHidden, applyPrimary, sessionId]);

  const refreshBases = useCallback(async () => {
    setLoading(true);
    try {
      // With the proof, for the reason `readSelection` gives: a daemon that
      // filters what an unproven caller may see would otherwise hand this list
      // back with the user's own private bases missing.
      const res = await listBases({ headers: await userActionHeaders(), throwOnError: true });
      setBases(res.data || []);
      setBasesLoaded(true);
      setBasesError(null);
    } catch (err) {
      // Keep the list we already had. A failed request is not a list of zero
      // bases, and a consumer reading it as one would conclude every base it
      // knows about is gone.
      console.error('listBases failed:', err);
      // …and say that it is stale, in a value that is never falsy on failure:
      // an error reported as '' reads as "no failure" at every call site.
      const message = err instanceof Error ? err.message : String(err);
      setBasesError(message || 'Could not load knowledge bases.');
    } finally {
      setLoading(false);
    }
  }, []);

  /**
   * A write that did not land: say so, then show the truth.
   *
   * The daemon's answer is re-read (with the proof, so a private chat is not
   * refused a second time for a different reason) and adopted. When even that
   * fails, the last selection the daemon CONFIRMED comes back from
   * `localStorage` — never the optimistic value, which is the one thing known
   * not to have been stored. The list is re-read too: the likeliest reason a
   * choice is refused is that the base it named went away in another window.
   */
  const recoverFromFailedWrite = useCallback(
    async (generation: number, failure: unknown) => {
      toastError({
        title: SELECTION_NOT_SAVED_TITLE,
        msg: briefSelectionFailure(failure),
      });
      void refreshBases();
      try {
        const data = await readSelection(sessionId);
        if (generation !== selectionGenerationRef.current) return;
        applyPrimary(readPrimary(data));
        const hidden = readHidden(data);
        if (hidden) applyHidden(hidden);
      } catch (err) {
        if (generation !== selectionGenerationRef.current) return;
        console.warn(
          'Knowledge selection not re-read after a failed write:',
          briefSelectionFailure(err)
        );
        restoreConfirmedSelection();
      }
    },
    [applyHidden, applyPrimary, refreshBases, restoreConfirmedSelection, sessionId]
  );

  /**
   * @param nextHiddenKbIds the set this edit establishes, or `null` to leave the
   * set alone — the daemon's "omit `hidden_kbs`". A pointer-only gesture must
   * use `null`: this chat may be *inheriting* the machine-wide hidden list, and
   * echoing the resolved list back would install a session-scope set override
   * the user never asked for.
   */
  const syncSelection = useCallback(
    (primary: PrimaryUpdate, nextHiddenKbIds: string[] | null) => {
      const generation = ++selectionGenerationRef.current;
      // Optimistic: show the caller's intent now, then adopt whatever the
      // daemon says it actually applied.
      if (primary.kind === 'set') setPrimaryKbIdState(primary.id);
      if (primary.kind === 'clear') setPrimaryKbIdState(null);
      // `inherit` gets no optimistic value at all. Which base the chat lands on
      // is resolved by the daemon — it re-reads the machine pointer and filters
      // it through this chat's set — so guessing here is guessing at the very
      // rule this gesture exists to defer to, and a guess that never lands is a
      // chat claiming to follow a default it does not.
      if (primary.kind === 'unchanged') {
        // A set-only edit can orphan the primary — hiding the primary's own
        // base is precisely the case the daemon's repair exists for. Until that
        // repair comes back the renderer must not keep naming a base this chat
        // no longer includes: IngestPanel passes `primaryKbId` straight into
        // `/knowledge/bases/<id>/ingest`, so a stale pointer aims a digest at a
        // base the user just removed. Drop the subject and adopt the daemon's
        // answer below. Only the in-memory pointer is dropped — the persisted
        // one stays until an authoritative answer replaces it, so a reload
        // during the in-flight window still has a last-known value to show.
        setPrimaryKbIdState((current) =>
          current && nextHiddenKbIds?.includes(current) ? null : current
        );
      }
      if (nextHiddenKbIds) setHiddenKbIdsState(nextHiddenKbIds);
      // ⚠ **Nothing is written to `localStorage` here.** It used to take the
      // optimistic value before the POST went out, so a write the daemon refused
      // left it holding a selection the daemon never stored — and a later
      // hydrate that could not reach the daemon then restored that guess as if
      // it were the chat's (QA 2026-09-10 F14). `applyPrimary`/`applyHidden`
      // persist the daemon's ANSWER, and only they do.
      writesInFlightRef.current += 1;
      void (async () => {
        let data: SelectionPayload = undefined;
        let failure: unknown = null;
        try {
          const res = await setActive({
            // Issue #56 Task 58: this POST names a chat, and repointing a
            // PRIVATE chat's knowledge bases needs the proof-of-user.
            // `userActionHeaders` resolves to `{}` rather than rejecting when
            // there is no bridge, and the daemon then refuses in words.
            headers: await userActionHeaders(),
            body: {
              // The three primary gestures are mutually exclusive on the wire:
              // two of them in one body is a 400 naming both fields, not a
              // precedence rule. At most one of these is ever true.
              primary_kb: primary.kind === 'set' ? primary.id : undefined,
              clear_primary: primary.kind === 'clear',
              inherit_primary: primary.kind === 'inherit',
              hidden_kbs: nextHiddenKbIds ?? undefined,
              session_id: sessionId || undefined,
            },
            throwOnError: false,
          });
          data = res?.data;
          // An error envelope instead of a selection is a write that did not
          // land, however politely it arrived.
          if (!data) failure = res?.error ?? 'The daemon answered without a selection.';
        } catch (err) {
          failure = err;
        } finally {
          writesInFlightRef.current -= 1;
        }
        // A newer edit is already in flight (or has already answered): this
        // answer describes a selection the user has clicked past, so applying
        // it — or reporting it — would silently undo their newer choice.
        if (generation !== selectionGenerationRef.current) return;
        if (failure !== null) {
          // Keeping the optimistic value would leave the chip, the Knowledge
          // view and the ingest target describing a selection the daemon never
          // applied, with nothing to correct it and nothing on screen to say so.
          void recoverFromFailedWrite(generation, failure);
          return;
        }
        // The daemon owns the "primary must be a member" repair: hiding the
        // primary promotes to the first remaining base, hiding everything
        // clears it. Adopt its answer instead of re-implementing that rule
        // here, where the two would silently drift apart.
        applyPrimary(readPrimary(data));
        // Adopt the set too, not just the pointer: the repair moves both, and
        // taking one half of it is how the renderer ends up holding a primary
        // that is not a member of its own visible set.
        const appliedHidden = readHidden(data);
        if (appliedHidden) applyHidden(appliedHidden);
      })();
    },
    [applyHidden, applyPrimary, recoverFromFailedWrite, sessionId]
  );

  const setPrimaryKbId = useCallback(
    (id: string | null) => {
      // The primary must be a member of the set, so making a HIDDEN base
      // primary adds it to this chat in the same request — one gesture, one
      // POST, validated by the daemon against the state it produces.
      //
      // ⚠ Only then does the set travel. A base already in the chat needs no
      // set edit, and echoing the resolved hidden list back would install a
      // session-scoped override on a chat that is inheriting the machine-wide
      // list — the very thing `syncSelection`'s `null` exists to avoid. It did
      // exactly that on every "make primary" until 2026-09-11.
      const nextHidden =
        id && hiddenKbIds.includes(id) ? hiddenKbIds.filter((hiddenId) => hiddenId !== id) : null;
      syncSelection(id ? { kind: 'set', id } : { kind: 'clear' }, nextHidden);
    },
    [hiddenKbIds, syncSelection]
  );

  /**
   * Re-read the machine-wide default primary — `GET /active` with no
   * `session_id`. It is not derivable from anything the chat-scoped answer
   * carries: a chat that pins alpha and a chat that inherits an alpha default
   * report the identical selection.
   */
  const refreshDefaultPrimary = useCallback(async () => {
    // This state only drives a chat's "follow the default" affordance. At
    // machine scope the active response already resolves the product default
    // (Soul), and there is no chat override to compare with it.
    if (!sessionId) {
      setDefaultPrimaryKbId(null);
      return;
    }
    try {
      setDefaultPrimaryKbId(readPrimary(await readSelection(null)));
    } catch (err) {
      // Keep the last known default: a failed read is not evidence that there
      // is none, and inventing one would offer a base nobody chose.
      console.warn('Machine-wide knowledge default not read:', briefSelectionFailure(err));
    }
  }, [sessionId]);

  const followDefaultPrimary = useCallback(() => {
    // This control only drops a chat override. Restoring the product default at
    // machine scope is a separate settings/CLI/API gesture, so this chat-only
    // affordance remains gated on `canFollowDefaultPrimary`.
    if (!sessionId) return;
    // The set travels separately and is deliberately left alone: a gesture that
    // means "stop overriding the pointer here" must not install a set override
    // on the way past.
    syncSelection({ kind: 'inherit' }, null);
  }, [sessionId, syncSelection]);

  const setHiddenKbIds = useCallback(
    (ids: string[]) => {
      const nextIds = Array.from(new Set(ids)).sort();
      // A set-only edit never states a primary — see PrimaryUpdate above.
      syncSelection({ kind: 'unchanged' }, nextIds);
    },
    [syncSelection]
  );

  const toggleKbHidden = useCallback(
    (id: string) => {
      const nextIds = hiddenKbIds.includes(id)
        ? hiddenKbIds.filter((hiddenId) => hiddenId !== id)
        : [...hiddenKbIds, id];
      setHiddenKbIds(nextIds);
    },
    [hiddenKbIds, setHiddenKbIds]
  );

  const hideAllKnowledgeBases = useCallback(() => {
    setHiddenKbIds(bases.map((base) => base.id));
  }, [bases, setHiddenKbIds]);

  const showAllKnowledgeBases = useCallback(() => {
    setHiddenKbIds([]);
  }, [setHiddenKbIds]);

  // `refresh` stays referentially stable across chat switches: half a dozen
  // consumers run it from an effect keyed on its identity, and one of them (the
  // manager dialog) resets a half-typed form whenever that effect re-runs.
  const resyncSelectionRef = useRef(resyncSelection);
  resyncSelectionRef.current = resyncSelection;

  /**
   * Re-read the daemon: the base list and this scope's selection, together.
   *
   * This is the Knowledge feature's one change signal — the view calls it when
   * it mounts, the picker and the manager when they open, every create, delete,
   * rename and import when it lands, and the provider itself when a turn ends
   * (below). The selection rides with the list because anything that moved one
   * can have moved the other: deleting a base clears every pointer that named
   * it, and the agent can create a base and pin it in one turn.
   */
  const refresh = useCallback(async () => {
    await Promise.all([refreshBases(), resyncSelectionRef.current()]);
  }, [refreshBases]);

  const registerGraphRefresh = useCallback((fn: (() => Promise<void>) | null) => {
    graphRefreshRef.current = fn;
  }, []);

  const triggerGraphRefresh = useCallback(() => {
    void graphRefreshRef.current?.();
  }, []);

  useEffect(() => {
    // The list only: the hydrate below owns the selection on mount, and a
    // second read racing it would be answered by the same daemon twice.
    void refreshBases();
  }, [refreshBases]);

  useEffect(() => {
    // QA 2026-09-10 F6. The agent creates, deletes and merges knowledge bases in
    // a chat — directly, or from inside `execute_code`, where no knowledge tool
    // call is visible to the renderer at all — and before this nothing told the
    // composer's chip, which read "2 visible" over three bases until a remount.
    // `message-stream-finished` is the app's existing end-of-turn signal: the
    // sidebar, the extension chip and the tool count already re-read on it, so
    // this is one more reader of a signal rather than a second subscription.
    // Mounted with the provider, once per renderer — a subscription belongs to a
    // mount, not to a lookup.
    const onTurnFinished = () => void refresh();
    window.addEventListener('message-stream-finished', onTurnFinished);
    return () => window.removeEventListener('message-stream-finished', onTurnFinished);
  }, [refresh]);

  useEffect(() => {
    const local = localStorage.getItem(storageKey);
    let localHidden: string[] = [];
    try {
      const rawHidden = localStorage.getItem(hiddenStorageKey);
      if (rawHidden) {
        const parsed = JSON.parse(rawHidden);
        if (Array.isArray(parsed)) {
          localHidden = parsed.filter((id): id is string => typeof id === 'string');
        }
      }
    } catch {
      localHidden = [];
    }
    setPrimaryKbIdState(local);
    setHiddenKbIdsState(localHidden);
    // The default belongs to whoever asks for it next. Carrying the previous
    // chat's read across a switch would offer the new chat a way back to a
    // pointer nobody has re-checked — and at machine scope, to one that does
    // not exist.
    setDefaultPrimaryKbId(null);
    let cancelled = false;
    // The hydrate takes a generation of its own: switching sessions must not
    // leave a write from the previous one able to answer into the new one.
    const generation = ++selectionGenerationRef.current;

    void (async () => {
      try {
        const data = await readSelection(sessionId);
        if (cancelled || generation !== selectionGenerationRef.current) return;
        applyPrimary(readPrimary(data));
        applyHidden(readHidden(data) ?? []);
      } catch (err) {
        if (cancelled) return;
        // ⚠ One quiet line, deliberately: the gate's refusal is ~900
        // characters addressed to an AI agent, and its first sentence is all a
        // person reading the console needs (`selectionWarning.ts`).
        //
        // ⚠ This comment used to call that refusal "a correct outcome" for "a
        // private chat opened while a public model is bound". It was neither:
        // the read carried no proof, so the daemon refused it for EVERY private
        // chat whatever model was bound, and the Knowledge view then showed this
        // renderer's cache as the chat's selection (QA 2026-09-10 F14). With the
        // proof on the read, a refusal lands here only for a caller that
        // genuinely has neither the proof nor a private model — a daemon started
        // without a user-action key — and only there is falling back right.
        console.warn('Knowledge selection not hydrated:', briefSelectionFailure(err));
        setPrimaryKbIdState(local);
        setHiddenKbIdsState(localHidden);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [applyHidden, applyPrimary, hiddenStorageKey, sessionId, storageKey]);

  // A pointer at a base the list does not hold is shown as no primary — the
  // view, the ingest target and the graph must never aim at a base that is gone
  // — but it is NOT written back as one. The daemon already cleared every
  // pointer the delete touched (D2), and a durable "no primary" derived from
  // this renderer's list would override a chat that merely inherits, from a
  // list that may be the stale one. `refresh` re-reads the truth instead.
  const primaryKbId =
    basesLoaded && storedPrimaryKbId && !bases.some((b) => b.id === storedPrimaryKbId)
      ? null
      : storedPrimaryKbId;
  const primaryKb = useMemo(
    () => bases.find((b) => b.id === primaryKbId) ?? null,
    [bases, primaryKbId]
  );
  const visibleBases = useMemo(
    () => bases.filter((base) => !hiddenKbIds.includes(base.id)),
    [bases, hiddenKbIds]
  );
  const defaultPrimaryKb = useMemo(
    () => bases.find((b) => b.id === defaultPrimaryKbId) ?? null,
    [bases, defaultPrimaryKbId]
  );
  const canFollowDefaultPrimary = useMemo(
    () =>
      // A chat already showing the default has nothing to inherit…
      defaultPrimaryKb !== null &&
      defaultPrimaryKb.id !== primaryKbId &&
      // …and a default this chat has left out of its set is not a member of the
      // set it would be primary of, so inheriting it resolves to no primary at
      // all. Offering a click that visibly does nothing is worse than offering
      // none: the membership switch is the gesture for that, and turning it on
      // is what makes this offer appear.
      !hiddenKbIds.includes(defaultPrimaryKb.id),
    [defaultPrimaryKb, hiddenKbIds, primaryKbId]
  );

  const value: KnowledgeContextType = {
    bases,
    visibleBases,
    loading,
    basesError,
    primaryKb,
    primaryKbId,
    hiddenKbIds,
    defaultPrimaryKb,
    canFollowDefaultPrimary,
    refreshDefaultPrimary,
    followDefaultPrimary,
    setPrimaryKbId,
    setHiddenKbIds,
    toggleKbHidden,
    hideAllKnowledgeBases,
    showAllKnowledgeBases,
    refresh,
    registerGraphRefresh,
    triggerGraphRefresh,
  };

  return <KnowledgeContext.Provider value={value}>{children}</KnowledgeContext.Provider>;
}

export function useKnowledge(): KnowledgeContextType {
  const ctx = useContext(KnowledgeContext);
  if (!ctx) throw new Error('useKnowledge must be used inside <KnowledgeProvider>');
  return ctx;
}
