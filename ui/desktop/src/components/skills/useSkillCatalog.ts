/**
 * The skill catalog, as the daemon serves it (#113).
 *
 * # What this replaces
 *
 * Every skill surface used to walk the filesystem itself, over
 * `ALL_SKILL_DIRS` — three roots. The backend discovers seven kinds, including
 * `~/.config/biorouter/extensions/<name>/skills`, so a skill bundled inside an
 * installed extension (BiorOffice's Word/Excel/PowerPoint, MarkItDown's
 * converter) was live for the model and had no row in the picker. A second
 * scanner with a different root list is not a bug you fix once; the lists drift
 * again the next time a root is added. So there is no scanner here — the
 * catalog is fetched.
 *
 * # The rule this hook exists to enforce
 *
 * **A switch reflects confirmed backend state, never local intent.** A per-chat
 * toggle used to write a `Map` in React state and raise a green toast. Nothing
 * left the renderer. Here the toggle is optimistic *for one frame* and then
 * replaced by the catalog the daemon returns; a failed write restores the
 * previous catalog and reports the error, and no success toast is raised in
 * that case. `applyResultIsAuthoritative` in the tests pins it.
 *
 * # Two scopes, deliberately different destinations
 *
 * * **Hub** (no session): the machine-wide preference, `skills-config.json`,
 *   through the existing `skillOverrides` store — the same file
 *   `biorouter skill enable/disable` writes. The catalog is then refetched
 *   rather than patched, because that file is read fresh on every catalog view.
 * * **A chat** (session id present): `POST /skills/session`, which persists
 *   `workspace_skills/v1` on the session row and touches no machine-wide file.
 *   Getting this backwards would make one chat's toggle change every other
 *   chat, window and CLI invocation.
 *
 * # `catalog:changed` (#112)
 *
 * Installing an extension can add a whole skill root
 * (`~/.config/biorouter/extensions/<name>/skills`), so this hook subscribes to
 * the machine-wide `catalog:changed` event and rescans on it.
 *
 * ⚠ **It keys off `revision` and reads nothing else from the payload.** The
 * event carries a `skills[]` list, and a consumer that repaired its inventory
 * from that list would drift the first time two events raced. A monotonic
 * revision that has advanced means "you are stale"; the answer to being stale
 * is to refetch.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { CatalogBundle, CatalogSkill, CatalogView, SkillRoot } from '../../api';
import { refreshSkillCatalog, setSessionSkills, skillCatalogHandler } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';
import { isContextBundle, isContextSkill } from '../settings/contexts/contexts';
import {
  loadSkillOverrides,
  saveSkillOverrides,
  setSkillOverride,
} from '../../store/skillOverrides';

/** What one mutation did, with the daemon's own words when it refused. */
export type SkillMutationResult = { ok: true } | { ok: false; error: string };

/** One row in a skill picker: a standalone skill, or a whole bundle. */
export type SkillCatalogEntry =
  | { kind: 'single'; key: string; skill: CatalogSkill; enabled: boolean }
  | { kind: 'bundle'; key: string; bundle: CatalogBundle; enabled: boolean };

/** Physical row identity. Bundle names are only logical toggle identities. */
export function bundleCatalogEntryKey(bundle: CatalogBundle): string {
  return JSON.stringify([bundle.sourceRoot, bundle.name]);
}

/** The backend intentionally applies a bundle-name toggle across every source. */
export function skillCatalogToggleKey(entry: SkillCatalogEntry): string {
  return entry.kind === 'single' ? entry.skill.name : entry.bundle.name;
}

export interface SkillCatalogState {
  /** Rows for the picker: bundles first, then standalone skills, each sorted. */
  entries: SkillCatalogEntry[];
  /** Every skill, including bundle members — for search and for detail rows. */
  skills: CatalogSkill[];
  /**
   * Bundles a picker may show — Contexts already removed.
   *
   * ⚠ Filtered, unlike `skills`. An unfiltered list here is a loaded gun
   * sitting beside the `pickerBundles` helper that exists *because* three call
   * sites forgot to filter; nothing reads this field today, and the next thing
   * that does should not have to know.
   */
  bundles: CatalogBundle[];
  roots: SkillRoot[];
  /** Bumped by the daemon on every rescan. */
  generation: number;
  loading: boolean;
  /** Set when the catalog could not be read at all. */
  error: string | null;
  /** Refetch. Pass `true` to make the daemon rescan the filesystem first. */
  reload: (rescan?: boolean) => Promise<void>;
  /**
   * Enable or disable one or more entries by key.
   *
   * ⚠ **The refusal message is carried in the RESULT, not in hook state.** It
   * was a `lastError` field once, and the caller read it immediately after
   * awaiting — from the render closure it had captured *before* the state
   * update, so every error toast said "The change was not saved." with the
   * reason silently dropped. A value that is only ever read by the awaiting
   * caller belongs to that call.
   *
   * Never throws, so a caller cannot forget the failure branch and leave a
   * stale switch on screen.
   */
  setEnabled: (keys: string[], enabled: boolean) => Promise<SkillMutationResult>;
}

const EMPTY: CatalogView = { generation: 0, roots: [], skills: [], bundles: [] };

/**
 * The machine-wide inventory-changed signal (#112). Owned by
 * `utils/catalogSubscription`, which is the module that *dispatches* it — this
 * file used to declare its own `'catalog:changed'` literal, so one contract had
 * two declarations that were equal only by luck. Re-exported rather than merely
 * imported because `useSkillCatalog.test.ts` reads the name from here.
 */
export { CATALOG_CHANGED_EVENT };

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return 'the request failed.';
}

/**
 * Contexts ship with the app, so they are not what a user means by "skills".
 * Filtered here rather than in the daemon, because the catalog is also what the
 * importer and the Settings **Contexts** pane read, and those legitimately want
 * the full set. (Settings -> Skills reads `entries`, so it is filtered too —
 * deliberately: a Context's switch lives in the Contexts pane.)
 *
 * ⚠ **`skill.bundle` is passed as a belt to the braces, not as the fix.** Both
 * call sites test `!skill.bundle` first, so today a member never reaches this
 * function with a bundle set — deleting the argument would fail no test, and
 * the comment saying otherwise would have been a lie. What it buys is the case
 * those two clauses do not cover between them: a member the daemon reports with
 * a bundle from a surface that has not filtered members out. The bundle row
 * itself is filtered by `pickerBundles`, which IS load-bearing.
 */
function isPickerSkill(skill: CatalogSkill): boolean {
  return !isContextSkill(skill.name, skill.bundle);
}

/**
 * Bundle rows for a picker — every bundle except the ones that are Contexts.
 *
 * ⚠ **Exported, and used by all three pickers.** Bundles reached the composer,
 * the `@`-mention list and the workflow resource picker through three separate
 * unfiltered `view.bundles` reads. One helper, three callers: a Context bundle
 * cannot be filtered out of two of them and left in the third. The composer one
 * matters most — its rows feed "Enable all", which writes `skills-config.json`,
 * the one file `contexts.ts` forbids a Context from reaching.
 */
export function pickerBundles(view: CatalogView): CatalogBundle[] {
  return view.bundles.filter((bundle) => !isContextBundle(bundle.name));
}

/**
 * The catalog once, for a caller that is not a React component.
 *
 * Same endpoint, same answer as the hook. It exists so a one-shot loader — the
 * `@`-mention list, the workflow resource picker — does not have to keep its
 * own filesystem scan around, which is what left both of them blind to
 * extension-bundled skills.
 */
export async function fetchSkillCatalog(sessionId?: string | null): Promise<CatalogView> {
  const response = await skillCatalogHandler<true>({
    query: sessionId ? { session_id: sessionId } : {},
    throwOnError: true,
  });
  return response.data;
}

/** Standalone skills — bundle members are reached through their bundle. */
export function standaloneSkills(view: CatalogView): CatalogSkill[] {
  return view.skills.filter((skill) => !skill.bundle && isPickerSkill(skill));
}

export function useSkillCatalog(sessionId: string | null): SkillCatalogState {
  const [view, setView] = useState<CatalogView>(EMPTY);
  // ⚠ **The rollback target is a ref, not the render closure's `view`.**
  // Two toggles can be made before React re-renders, and a callback created in
  // the first render captures the catalog as it was *then*. So when the second
  // toggle was refused, the rollback restored the state from before the FIRST —
  // silently undoing a change the daemon had already accepted, on screen only.
  // A ref written wherever the catalog is committed is always the last thing
  // this hook actually published, whatever the render timing.
  const committed = useRef<CatalogView>(EMPTY);
  const commit = useCallback((next: CatalogView) => {
    committed.current = next;
    setView(next);
  }, []);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Serialises mutations so two fast clicks cannot land out of order and leave
  // the switch showing the older of the two answers.
  const queue = useRef<Promise<unknown>>(Promise.resolve());

  const reload = useCallback(
    async (rescan = false) => {
      setLoading(true);
      try {
        const query = sessionId ? { session_id: sessionId } : {};
        const response = rescan
          ? await refreshSkillCatalog<true>({ query, throwOnError: true })
          : await skillCatalogHandler<true>({ query, throwOnError: true });
        commit(response.data);
        setError(null);
      } catch (err) {
        setError(`Could not read the skill catalog: ${errorText(err)}`);
      } finally {
        setLoading(false);
      }
    },
    [commit, sessionId]
  );

  useEffect(() => {
    void reload();
  }, [reload]);

  // A `window` event rather than React state, so this works from any surface —
  // including one outside the provider tree, or in a second window.
  const appliedRevision = useRef(0);
  useEffect(() => {
    const onCatalogChanged = (event: Event) => {
      const revision = (event as CustomEvent<{ revision?: number }>).detail?.revision;
      if (typeof revision === 'number') {
        if (revision <= appliedRevision.current) return;
        appliedRevision.current = revision;
      }
      void reload(true);
    };
    window.addEventListener(CATALOG_CHANGED_EVENT, onCatalogChanged);
    return () => window.removeEventListener(CATALOG_CHANGED_EVENT, onCatalogChanged);
  }, [reload]);

  const setEnabled = useCallback(
    (keys: string[], enabled: boolean): Promise<SkillMutationResult> => {
      if (keys.length === 0) return Promise.resolve({ ok: true });

      const run = async (): Promise<SkillMutationResult> => {
        const previous = committed.current;
        // Optimistic, so the switch does not lag the click — and reverted below
        // if the write is refused.
        commit(applyOptimistically(previous, keys, enabled, sessionId !== null));

        try {
          if (sessionId) {
            // With the user's proof: a private chat refuses this write to a
            // caller without it (issue #56, QA 2026-09-10).
            const response = await setSessionSkills<true>({
              body: {
                sessionId,
                add: enabled ? keys : [],
                remove: enabled ? [] : keys,
              },
              headers: await userActionHeaders(),
              throwOnError: true,
            });
            commit(response.data.catalog);
          } else {
            keys.forEach((key) => setSkillOverride(key, enabled));
            await saveSkillOverrides();
            // `skills-config.json` is read fresh on every catalog view, so a
            // plain refetch already reflects the write — no rescan needed.
            const response = await skillCatalogHandler<true>({
              query: {},
              throwOnError: true,
            });
            commit(response.data);
          }
          return { ok: true };
        } catch (err) {
          commit(previous);
          if (!sessionId) {
            // Put the in-memory store back in step with the file we failed to
            // write, or the next save would persist this failed edit.
            await loadSkillOverrides();
          }
          return { ok: false, error: errorText(err) };
        }
      };

      const next = queue.current.then(run, run);
      queue.current = next;
      return next;
    },
    // ⚠ No `view` here — see `committed`. Depending on it would also give
    // `setEnabled` a new identity on every catalog change, which every caller
    // then has in ITS dependency array.
    [commit, sessionId]
  );

  const entries = useMemo((): SkillCatalogEntry[] => {
    const bundles: SkillCatalogEntry[] = pickerBundles(view)
      .map((bundle) => ({
        kind: 'bundle' as const,
        key: bundleCatalogEntryKey(bundle),
        bundle,
        enabled: bundle.state.effective,
      }))
      .sort((a, b) => a.bundle.displayName.localeCompare(b.bundle.displayName));

    const singles: SkillCatalogEntry[] = view.skills
      .filter((skill) => !skill.bundle && isPickerSkill(skill))
      .map((skill) => ({
        kind: 'single' as const,
        key: skill.name,
        skill,
        enabled: skill.state.effective,
      }))
      .sort((a, b) => a.skill.name.localeCompare(b.skill.name));

    return [...bundles, ...singles];
  }, [view]);

  return {
    entries,
    skills: view.skills,
    bundles: pickerBundles(view),
    roots: view.roots,
    generation: view.generation,
    loading,
    error,
    reload,
    setEnabled,
  };
}

/**
 * The one-frame optimistic view.
 *
 * ⚠ It edits `state.effective` **only**. `machineEnabled` and `session` are the
 * daemon's answer about where the change was persisted, and guessing at them
 * would let the interface claim a per-chat toggle had changed the machine-wide
 * preference. The authoritative catalog replaces this a moment later either
 * way; this exists so the switch does not visibly lag the pointer.
 *
 * Exported for the tests, which assert exactly that restriction.
 */
export function applyOptimistically(
  view: CatalogView,
  keys: string[],
  enabled: boolean,
  scopedToSession: boolean
): CatalogView {
  const targeted = new Set(keys);
  const bundleNames = new Set(
    view.bundles.filter((bundle) => targeted.has(bundle.name)).map((bundle) => bundle.name)
  );
  const hit = (name: string, bundle?: string | null) =>
    targeted.has(name) || (bundle != null && bundleNames.has(bundle));

  return {
    ...view,
    skills: view.skills.map((skill) =>
      hit(skill.name, skill.bundle)
        ? {
            ...skill,
            state: {
              ...skill.state,
              effective: enabled,
              // A per-chat toggle cannot move the machine-wide answer, and a
              // machine-wide one cannot invent a per-chat deviation.
              machineEnabled: scopedToSession ? skill.state.machineEnabled : enabled,
            },
          }
        : skill
    ),
    bundles: view.bundles.map((bundle) =>
      targeted.has(bundle.name)
        ? {
            ...bundle,
            state: {
              ...bundle.state,
              effective: enabled,
              machineEnabled: scopedToSession ? bundle.state.machineEnabled : enabled,
            },
          }
        : bundle
    ),
  };
}
