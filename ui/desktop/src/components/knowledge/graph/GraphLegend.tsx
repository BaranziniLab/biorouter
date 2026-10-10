// ui/desktop/src/components/knowledge/graph/GraphLegend.tsx
import { useCallback, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, ChevronUp } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { knowledgeCopy } from '../copy';
import { GRAPH_PALETTE, typeFill } from '../../../styles/graphPalette';
import type { GraphCredibilityKey, GraphMode } from '../../../styles/graphPalette';
import { isHollowType } from './nodeMark';
import { CredibilityRing } from './CredibilityRing';
import { toggle, UNTYPED_KEY } from './graphFacets';
import type { FacetState } from './graphFacets';
import type { GraphModel } from './graphModel';

/**
 * The legend dock (ui-spec §4.7).
 *
 * ⚠ **Four credibility entries, not seven, and that is DR-9b's honesty clause
 * made visible.** The canvas draws four *distinguishable* ring treatments — four
 * arcs, one arc, dashed, solid-with-`!` — because a 1.6px stroke subtends 2–3
 * arcmin and the seven ring hues collapse to ΔE00 1.13 under tritanopia. The old
 * legend listed seven treatments against a four-entry key, so `web` and
 * `personal` had no row at all. They are one category here — *not academic* —
 * and the exact tier lives in the inspector and the Source facet.
 *
 * ⚠ **Every chip is a real `<button>`, via `Badge asChild`.** Wrapping a badge
 * in a raw button would defeat D-15: the global focus rule paints
 * `background-color: var(--background-focus)` on the focused control, and a
 * `tone="neutral"` badge is an OPAQUE `bg-background-medium` that covers it
 * completely — a keyboard user would get no focus indication on any legend chip.
 * With `asChild` the chip *is* the button and takes the focus surface directly.
 */

/** The four treatments the canvas actually distinguishes, in ring order. */
const CREDIBILITY_KEY: { key: GraphCredibilityKey; label: string }[] = [
  { key: 'peer_reviewed', label: 'Well sourced' },
  { key: 'gray_lit', label: 'Weakly sourced' },
  { key: 'web', label: 'Not academic' },
  { key: 'retracted', label: 'Retracted' },
];

/* ⚠ The dock's single `expanded` flag and its localStorage key are GONE. It
   toggled between two states that showed DISJOINT information — an inert row of
   28 unlabelled swatches, or a named grid with no evidence key — so persisting
   which one you were in persisted which half of the legend you could not see.
   Each section now discloses independently and every channel is present in
   both the rail and the card. */

interface Props {
  model: GraphModel;
  mode: GraphMode;
  facets: FacetState;
  onChange: (next: FacetState) => void;
  /**
   * `rail` is the permanent right column at the widest step; `popover` is the
   * same content inside the filter bar's `Legend` popover below it. Same
   * component either way — there is deliberately not a second legend
   * implementation for the narrow case, and deliberately no canvas overlay: a
   * legend that covers the graph is the complaint this redesign exists to fix.
   */
  variant?: 'rail' | 'popover';
}

export function GraphLegend({ model, mode, facets, onChange, variant = 'rail' }: Props) {
  const palette = GRAPH_PALETTE[mode];

  /**
   * The families actually present, in ladder order — and `null` in OKF mode.
   *
   * In OKF there are no families: every node takes the hashed fallback and the
   * circle, so the legend lists the types present by count instead. A family
   * heading over one hashed type would teach a mapping that does not exist.
   */
  const families = useMemo(() => {
    const present = new Set(model.typeCounts.map((t) => t.type));
    const rows = Object.entries(palette.families)
      .map(([name, family]) => ({
        name,
        members: family.members.filter((m) => present.has(m)),
      }))
      .filter((f) => f.members.length > 0);
    return rows.length > 0 ? rows : null;
  }, [model, palette]);

  const flatTypes = useMemo(() => model.typeCounts.slice(0, 24), [model]);

  const toggleType = useCallback(
    (type: string) => onChange({ ...facets, types: toggle(facets.types, type) }),
    [facets, onChange]
  );

  const toggleFamily = useCallback(
    (members: string[]) => {
      const allOn = members.every((m) => facets.types.has(m));
      const next = new Set(facets.types);
      for (const m of members) {
        if (allOn) next.delete(m);
        else next.add(m);
      }
      onChange({ ...facets, types: next });
    },
    [facets, onChange]
  );

  const chip = (type: string) => (
    <Badge
      key={type}
      asChild
      variant="chip"
      tone="neutral"
      // The pressed state is a translucent tint, never `tone="accent"`: the
      // focus fill has to read THROUGH it, and an opaque accent tone would
      // reintroduce the very problem `asChild` exists to solve.
      // ⚠ **`h-5` overrides the chip role's 24px, and only here.** 28 types in
      // a 339px rail wrap to 12 rows; at the shared height those rows alone are
      // 336px of a 486px column, which is what put `Provenance & context` below
      // the fold. The role height is right for a chip you hunt for in a filter
      // bar and wrong for a dense key you read as a block — this is the one
      // surface that lists every type at once. `badge.tsx` keeps its 24px for
      // everyone else; nothing shared moves.
      className={`h-5 ${facets.types.has(type) ? 'tint-selected tint-interactive' : ''}`}
    >
      <button type="button" aria-pressed={facets.types.has(type)} onClick={() => toggleType(type)}>
        {/* ⚠ **The swatch must agree with the MARK.** Provenance & Context is
            drawn hollow on the canvas (R-04), so its key is drawn hollow too —
            at a 1.5px inset ring rather than the 2.5px a first draft used,
            which on a 10px swatch left a hole and read as a ring rather than a
            mark. A key that disagrees with the mark teaches the wrong thing. */}
        <span
          aria-hidden="true"
          className="br-swatch-ring h-2.5 w-2.5 shrink-0 rounded-inner"
          style={
            isHollowType(type, mode)
              ? { boxShadow: `inset 0 0 0 1.5px ${typeFill(type, mode)}` }
              : { background: typeFill(type, mode) }
          }
        />
        {type}
      </button>
    </Badge>
  );

  if (model.typeCounts.length === 0 && !model.hasExternal) return null;

  return (
    <div
      data-testid="knowledge-graph-legend"
      className={
        variant === 'popover'
          ? // ⚠ **The cap is what Radix MEASURED, not a guess.** A flat
            // `max-h-[26rem]` clipped `Provenance & Context` mid-list at a 792px
            // viewport with 100px of room still free below the popover — the
            // same "below the fold by default" defect the rail had, on the
            // surface that replaces the rail at every step under 1400px.
            // `--radix-popover-content-available-height` is the distance to the
            // collision boundary, so the list grows into whatever the viewport
            // actually offers and still scrolls when it genuinely cannot fit.
            // The `26rem` stays as the fallback for the render before Radix has
            // measured, where the variable is undefined.
            //
            // ⚠ **The `-2px` is `PopoverContent`'s own border, and without it the
            // card sits exactly 2px off-screen.** Radix reports the height
            // available to the CONTENT box; this cap is on a child, so the
            // wrapper's 1px top and bottom border are spent twice. Measured, not
            // padded for luck — 522px of content in a 521.5px cap.
            'flex max-h-[calc(var(--radix-popover-content-available-height,26rem)-2px)] flex-col overflow-y-auto'
          : 'br-knowledge-detail flex min-h-0 flex-col overflow-y-auto border-l border-border-subtle bg-background-default'
      }
    >
      {/* ⚠ **The rail must fit its own height, and measured it did not.** At a
          1690x760 pane the rail held 856px of content in 544px, so
          `Provenance & context` — the family a reader is most likely to be
          looking up, because it is the one drawn hollow — sat below the fold by
          default. Two changes, both cheap: the sections are denser (see the
          `gap` values below), and `Evidence` opens CLOSED. Evidence is four
          rows explaining a ring treatment, useful once; node types are the key
          a reader returns to. Nothing is hidden — both disclose in their own
          heading. */}
      <LegendSection title="Node types">
        {(families ?? []).map((family) => (
          <div key={family.name} className="flex flex-col gap-0.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => toggleFamily(family.members)}
                  className="flex items-center gap-2 self-start rounded-inner text-caps leading-none text-text-muted"
                >
                  {family.name}
                </button>
              </TooltipTrigger>
              <TooltipContent side="left">
                {knowledgeCopy.graph.familyFilter(family.name)}
              </TooltipContent>
            </Tooltip>
            <div className="flex flex-wrap gap-1">{family.members.map(chip)}</div>
          </div>
        ))}
        {!families && (
          <div className="flex flex-wrap gap-1.5">{flatTypes.map((t) => chip(t.type))}</div>
        )}
        <ExtraRows model={model} mode={mode} facets={facets} onChange={onChange} />
      </LegendSection>

      {/* ⚠ **The evidence key is present in EVERY state now.** The dock had two
          states showing DISJOINT information: collapsed listed 28 unlabelled
          swatches and was entirely inert, expanded named them but dropped the
          credibility key altogether. A legend that omits a channel the canvas
          paints is worse than one that is merely small. */}
      <LegendSection title="Evidence" initiallyOpen={false}>
        <div className="flex flex-col gap-1.5">
          {CREDIBILITY_KEY.map((entry) => (
            <span key={entry.key} className="flex items-center gap-2">
              <CredibilityRing tier={entry.key} mode={mode} />
              <span className="text-supporting text-text-muted">{entry.label}</span>
            </span>
          ))}
        </div>
      </LegendSection>
    </div>
  );
}

/**
 * One collapsible section of the legend.
 *
 * ⚠ **The disclosure lives in the section's OWN heading**, and that is the fix
 * rather than a style choice. The dock put a single Expand/Collapse control at
 * the end of a `flex` row with `ml-auto`, so when the content overflowed —
 * which it did — `ml-auto` resolved to 0 and the only way to collapse the
 * legend was to scroll to the end of the thing you were trying to collapse.
 */
function LegendSection({
  title,
  children,
  initiallyOpen = true,
}: {
  title: string;
  children: ReactNode;
  initiallyOpen?: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  return (
    <section className="border-b border-border-subtle px-3 py-1.5 last:border-b-0">
      <div className="mb-1 flex items-center gap-1">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center justify-between gap-2 rounded-inner text-caps text-text-muted"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
        >
          {title}
          {open ? (
            <ChevronUp aria-hidden="true" className="h-icon-row w-icon-row shrink-0" />
          ) : (
            <ChevronDown aria-hidden="true" className="h-icon-row w-icon-row shrink-0" />
          )}
        </button>
      </div>
      {open && <div className="flex flex-col gap-1.5">{children}</div>}
    </section>
  );
}

/** `External` and `Unrecognised type` — shown only when the graph contains them. */
function ExtraRows({
  model,
  mode,
  facets,
  onChange,
}: {
  model: GraphModel;
  mode: GraphMode;
  facets: FacetState;
  onChange: (next: FacetState) => void;
}) {
  if (!model.hasExternal && !model.hasUnrecognisedTypes && !model.untyped) return null;
  return (
    <div className="flex flex-wrap items-center gap-3">
      {model.hasExternal && (
        <span className="flex items-center gap-2 text-supporting text-text-muted">
          <span
            aria-hidden="true"
            className="h-2 w-2 rounded-full border border-dashed border-text-muted opacity-45"
          />
          Referenced, no page yet
        </span>
      )}
      {model.hasUnrecognisedTypes && <Badge tone="warning">Unrecognized type</Badge>}
      {model.untyped && (
        <Badge
          asChild
          variant="chip"
          tone="neutral"
          className={facets.types.has(UNTYPED_KEY) ? 'tint-selected tint-interactive' : undefined}
        >
          <button
            type="button"
            aria-pressed={facets.types.has(UNTYPED_KEY)}
            onClick={() => onChange({ ...facets, types: toggle(facets.types, UNTYPED_KEY) })}
          >
            <span
              aria-hidden="true"
              className="br-swatch-ring h-2 w-2 shrink-0 rounded-full"
              style={{ background: typeFill('Other', mode) }}
            />
            Untyped
          </button>
        </Badge>
      )}
    </div>
  );
}
