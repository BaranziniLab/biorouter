import { useCallback, useEffect, useState } from 'react';
import { getKbTier, setKbTier, type KbTier } from '../../api';
import { toastError, toastSuccess } from '../../toasts';
import { userActionHeaders } from '../../utils/userAction';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import { DangerousConfirmDialog } from '../ui/DangerousConfirmDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { knowledgeCopy } from './copy';
import { useKnowledge } from './KnowledgeContext';

/**
 * Issue #56 DR-18 — the one surface that moves a knowledge base's privacy tier.
 *
 * ⚠ **The lowering direction is the dangerous one, and it is not symmetric with
 * a session's.** Declassifying a chat exposes one transcript the user is looking
 * at. Publicizing a base exposes *everything every private model ever wrote into
 * it*, to every public model, from the next tool call onward — and for anything
 * already read there is no undo. So the confirmation **counts what it is about
 * to release** rather than asking a generic question, and the counts come from
 * the daemon, which reads them off the tree.
 *
 * The grading is by DIRECTION, not by provenance (which is what grades
 * `DeclassifySessionDialog`), because a base has one direction that discloses
 * and one that does not:
 *
 * | Direction | Confirmation | Why |
 * |---|---|---|
 * | private → public | typed phrase = the **base id**, with the counts beside it | the id is short, unique, and forces the user to check *which* base — default names like `default` or `Notes` are the KB analogue of `fallback_session_name`'s duplicates |
 * | public → private | single click | nothing is disclosed; a phrase would be theatre and the reversal is one click away |
 *
 * **No 5-second undo on the publicize path.** `DeclassifySessionDialog` offers
 * one for `turn:*` chats because the exposure is a single transcript. Here the
 * first thing a public model does with a released base is read it, and an undo
 * that cannot recall what was read is a false promise.
 */
export interface KbTierControlKb {
  id: string;
  name: string;
  tier: KbTier;
  /** From `GET /knowledge/bases/{id}/tier`. Undefined until it arrives. */
  pageCount?: number;
  rawSourceCount?: number;
}

export interface KbTierControlProps {
  kb: KbTierControlKb;
  onSetTier: (id: string, tier: KbTier) => void;
  busy?: boolean;
}

/**
 * What a publicize would release, in words.
 *
 * ⚠ It never counts to zero from an absent number. A "0 pages" release of a
 * 214-page base is the one sentence a user would act on wrongly, and the counts
 * arrive from a separate request that can still be in flight — or have failed.
 */
function blastRadius(kb: KbTierControlKb): string {
  const parts: string[] = [];
  if (kb.pageCount !== undefined) {
    parts.push(`${kb.pageCount} ${kb.pageCount === 1 ? 'page' : 'pages'}`);
  }
  if (kb.rawSourceCount !== undefined) {
    parts.push(`${kb.rawSourceCount} ${kb.rawSourceCount === 1 ? 'raw source' : 'raw sources'}`);
  }
  if (parts.length === 0) {
    return 'every page and raw source in this knowledge base';
  }
  return parts.join(' and ');
}

export function KbTierControl({ kb, onSetTier, busy = false }: KbTierControlProps) {
  const [confirming, setConfirming] = useState(false);
  const isPrivate = kb.tier === 'private';
  const enforcementOff = !usePrivacyTiersEnabled();
  const hint = isPrivate ? knowledgeCopy.tier.privateHint : knowledgeCopy.tier.publicHint;

  // A dialog left open while the subject changes must not confirm against the
  // base the user is no longer looking at.
  useEffect(() => {
    setConfirming(false);
  }, [kb.id]);

  return (
    <>
      {/* THE TRIGGER, AND ONLY THE TRIGGER, MOVED (spec 3.12). It used to be a
          bordered box at the top of the Sources rail: a badge plus a "Make
          public" button, with its explanation in a native `title` on a span no
          keyboard could reach. Below the 860px step the rail hides behind the
          default Graph tab, so on a narrow pane the base's tier and its control
          were both off screen. Now the badge in the Knowledge band IS the
          control, Crew's `ClassificationBadge`: the tooltip explains, the
          hidden run gives a screen reader the same sentence, and pressing it
          opens the one tier action. Everything from `confirming` down (the typed
          id, the counts from the daemon, `onSetTier`) is unchanged.

          `PrivacyBadge`, not a bare `Badge`: it stays the app's one tier mark
          (R10), with its padlock, its measured ink pair and its "(enforcement
          off)" suffix, so a base's tier reads exactly as a chat's does. The
          button around it is transparent and draws its focus as an outline
          outside the chip (`.br-knowledge-tier`), because the badge's opaque
          fill would cover the D-15 focus surface. */}
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                data-testid="kb-tier-chip"
                data-privacy={kb.tier}
                disabled={busy}
                className="br-knowledge-tier"
              >
                <span aria-hidden="true" className="inline-flex">
                  <PrivacyBadge tier={kb.tier} />
                </span>
                <span className="sr-only">
                  {knowledgeCopy.tier.name(isPrivate ? 'Private' : 'Public', hint, enforcementOff)}
                </span>
              </button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="bottom">{hint}</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="start">
          <DropdownMenuItem
            onSelect={() => (isPrivate ? setConfirming(true) : onSetTier(kb.id, 'private'))}
          >
            {isPrivate ? knowledgeCopy.tier.makePublic : knowledgeCopy.tier.makePrivate}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {confirming && (
        <DangerousConfirmDialog
          open
          busy={busy}
          title={`Make "${kb.name}" public?`}
          description={
            `Every model, including public ones, will be able to read ${blastRadius(kb)}, ` +
            'including anything a private chat wrote into it. This cannot be undone for ' +
            'content that has already been read.'
          }
          // The ID, not the name. Knowledge bases routinely share a display name
          // (`Notes`, `Default`, an imported archive's), and the point of the
          // phrase is to make the user check WHICH base they are releasing.
          phrase={kb.id}
          fieldLabel="Type this knowledge base's ID to confirm"
          confirmLabel="Make public"
          onConfirm={() => {
            setConfirming(false);
            onSetTier(kb.id, 'public');
          }}
          onCancel={() => setConfirming(false)}
        >
          {/* The section's one pocket of pre-system classes, closed. `rounded-lg`
              was outside the radius ladder; `bg-background-muted/40` put an
              arbitrary alpha on an opaque surface STEP, which is a colour no
              token names; and `text-sm font-medium` / `text-xs` were the raw
              Tailwind spellings of `text-label` / `text-supporting`. */}
          <div className="rounded-element border border-transparent bg-background-fill p-3">
            <p className="truncate text-label text-text-default" title={kb.name}>
              {kb.name}
            </p>
            <p className="mt-0.5 font-mono text-supporting text-text-muted">{kb.id}</p>
          </div>
        </DangerousConfirmDialog>
      )}
    </>
  );
}

/**
 * The connected wrapper: reads the blast radius from the daemon, sends the
 * change with DR-16's proof-of-user, and refreshes the base list so every chip
 * in the app agrees afterwards.
 *
 * Split from {@link KbTierControl} so the confirmation's behaviour is testable
 * without a daemon — and so the component that renders the dangerous button has
 * no way to talk to the network on its own.
 */
export function KbTierPanel({ kb }: { kb: { id: string; name: string; tier: KbTier } }) {
  const { refresh } = useKnowledge();
  const [radius, setRadius] = useState<{ pageCount: number; rawSourceCount: number } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setRadius(null);
    void (async () => {
      try {
        const res = await getKbTier({
          path: { id: kb.id },
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        if (cancelled) return;
        setRadius({
          pageCount: res.data.page_count,
          rawSourceCount: res.data.raw_source_count,
        });
      } catch {
        // Leave the counts unknown. `blastRadius` then says "every page and raw
        // source" rather than inventing a number, which is the safe direction.
        if (!cancelled) setRadius(null);
      }
    })();
    return () => {
      cancelled = true;
    };
    // `kb.tier` too: a change moves the tier, and the counts are what the NEXT
    // confirmation will state.
  }, [kb.id, kb.tier]);

  const onSetTier = useCallback(
    (id: string, tier: KbTier) => {
      setBusy(true);
      void (async () => {
        try {
          await setKbTier({
            path: { id },
            body: { tier },
            // DR-16's proof-of-user. Without it the daemon refuses, correctly:
            // the server secret alone is reachable from any developer-enabled
            // agent shell (§9.3 A1) and is not evidence of a human.
            headers: await userActionHeaders(),
            throwOnError: true,
          });
          toastSuccess({
            title: tier === 'public' ? 'Knowledge base is public' : 'Knowledge base is private',
            msg:
              tier === 'public'
                ? 'Public models can now read it. The change is recorded.'
                : 'Biorouter now only lets a private model read or write it.',
          });
          await refresh();
        } catch (error) {
          toastError({
            title: 'Could not change this knowledge base',
            msg: error instanceof Error ? error.message : String(error),
          });
        } finally {
          setBusy(false);
        }
      })();
    },
    [refresh]
  );

  return (
    <KbTierControl
      kb={{
        id: kb.id,
        name: kb.name,
        tier: kb.tier,
        pageCount: radius?.pageCount,
        rawSourceCount: radius?.rawSourceCount,
      }}
      onSetTier={onSetTier}
      busy={busy}
    />
  );
}
