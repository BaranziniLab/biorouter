// ui/desktop/src/components/knowledge/graph/GraphFacetStrip.tsx
import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, Search, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { Command, CommandGroup, CommandInput, CommandItem, CommandList } from '../../ui/command';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { GRAPH_PALETTE, typeFill } from '../../../styles/graphPalette';
import type { GraphMode } from '../../../styles/graphPalette';
import { NodeSwatch } from './NodeSwatch';
import { GraphLegend } from './GraphLegend';
import { isHollowType } from './nodeMark';
import { toggle, UNTYPED_KEY } from './graphFacets';
import type { FacetState } from './graphFacets';
import type { GraphModel } from './graphModel';
import { knowledgeCopy } from '../copy';

/**
 * The facet strip (ui-spec §4.6).
 *
 * **OR within a facet, AND across facets**, and a node that fails takes the
 * search-miss alpha and stays in place — one dimming mechanism, never a second,
 * because "does not match my filter" and "is not related to what I am looking
 * at" are different states.
 *
 * ⚠ **Status is deliberately NOT a canvas channel.** The canvas already carries
 * four encodings (shape = family, fill = type, ring arcs = credibility,
 * dash/dot/taper = negated / synthesized / direction). A fifth is not readable
 * at a 6px node and every candidate collides with one that is already spoken
 * for. Status lives here and in the inspector; a filtered-out status dims.
 */

/** The fixed status vocabulary, so this one facet needs no search box. */
const STATUSES = ['draft', 'stable', 'deprecated', 'stale', 'retracted'] as const;

interface Props {
  model: GraphModel;
  mode: GraphMode;
  facets: FacetState;
  onChange: (next: FacetState) => void;
  /** How many nodes pass, for the `Showing N of M` readout. */
  passing: number;
  total: number;
  active: boolean;
}

export function GraphFacetStrip({ model, mode, facets, onChange, passing, total, active }: Props) {
  // Computed once and rendered TWICE — inline in the row, and again inside the
  // collapsed `More`/`Filters` menu (R-09). Deriving them at each call site
  // would be two sources of truth for one facet's options.
  const typeOptions: Option[] = [
    ...model.typeCounts.map((t) => ({
      value: t.type,
      label: t.type,
      count: t.count,
      fill: typeFill(t.type, mode),
      hollow: isHollowType(t.type, mode),
    })),
    // Counted, not hardcoded to 0. On a legacy base every other row carried a
    // count and this one silently did not, which reads as "no such pages"
    // rather than "the whole base".
    ...(model.untypedCount > 0 || model.typeCounts.length === 0
      ? [
          {
            value: UNTYPED_KEY,
            label: 'Untyped',
            count: model.untypedCount,
            fill: undefined,
          },
        ]
      : []),
  ];
  const predicateOptions: Option[] = model.predicateCounts.map((p) => ({
    value: p.predicate,
    label: p.predicate.replace(/^not_/, 'not ').replace(/_/g, ' '),
    count: p.count,
    negated: p.predicate.startsWith('not_'),
  }));
  const sourceOptions: Option[] = model.sourceOptions.map((sr) => ({
    value: sr.id,
    label: sr.label,
    count: sr.count,
  }));

  const typeFacet = (extraClass: string, testId?: string) => (
    <FacetCombobox
      label="Type"
      options={typeOptions}
      selected={facets.types}
      onToggle={(v) => onChange({ ...facets, types: toggle(facets.types, v) })}
      mode={mode}
      extraClass={extraClass}
      testId={testId}
    />
  );
  const predicateFacet = (extraClass: string, testId?: string) => (
    <FacetCombobox
      label="Predicate"
      options={predicateOptions}
      selected={facets.predicates}
      onToggle={(v) => onChange({ ...facets, predicates: toggle(facets.predicates, v) })}
      mode={mode}
      extraClass={extraClass}
      testId={testId}
    />
  );
  const sourceFacet = (extraClass: string, testId?: string) => (
    <FacetCombobox
      label="Source"
      options={sourceOptions}
      selected={facets.sources}
      onToggle={(v) => onChange({ ...facets, sources: toggle(facets.sources, v) })}
      mode={mode}
      extraClass={extraClass}
      testId={testId}
    />
  );
  const statusFacet = (extraClass: string, testId?: string) => (
    <StatusFacet facets={facets} onChange={onChange} extraClass={extraClass} testId={testId} />
  );

  return (
    <div
      data-testid="knowledge-graph-facets"
      // 44px, the band height every strip in the app shares (spec 3.12), not a
      // fourth height of its own.
      className="flex h-chrome flex-none items-center gap-2 border-b border-border-subtle bg-background-default px-4"
    >
      {/* ⚠ **NOTHING SCROLLS AND NOTHING WRAPS** (R-09). This row used to be an
          `overflow-x-auto` scroller, and the failure it produced is on record:
          with one facet active its 769px of content sat in a 550px box, so the
          two things the filter produced — `Showing N of M` and the only control
          that undoes it — were pushed off the right edge with no scrollbar
          affordance to say so. Pinning the readout outside the scroller fixed
          the symptom; it left the pickers themselves scrolling out of sight.

          They now DEGRADE BY PRIORITY instead, in three container-driven steps
          (see `.br-facet-*` in main.css). Search collapses last, because it is
          the only control that can reach a node whose type the user does not
          yet know. `Type` outlives the other three, because it is the one facet
          every base has — `Predicate` and `Source` are empty on a legacy base
          and `Status` only ever holds four values. What folds away carries its
          own count on the control that swallowed it, so a filter you cannot see
          is still reported; and `Clear` never leaves the row.

          Wrapping to a second line was rejected: it would take 48px from the
          canvas permanently, on exactly the pane sizes with the least canvas to
          give. */}
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <div className="relative flex-none">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute left-2 top-1/2 h-icon-row w-icon-row -translate-y-1/2 text-text-muted"
          />
          <Input
            type="text"
            data-testid="knowledge-graph-search"
            aria-label="Filter the graph by name or type"
            value={facets.search}
            onChange={(e) => onChange({ ...facets, search: e.target.value })}
            placeholder="Filter by name or type"
            className="w-[200px] pl-7"
          />
        </div>

        {/* ⚠ **Only `Type` is core, and `Predicate` had to give up that slot.**
            Measured at a 946px pane: search (200) + Type (76) + Predicate (105)
            + More (78) + Legend (93) plus gaps is 576px in a 532px column, so
            the row overflowed by 52px and clipped `Legend` mid-word. The cause
            is a seam between two steps that each look right alone — 940px
            WIDENS the sources rail, taking width from the centre column, while
            the filter row sheds nothing until the full-filters step. Shedding the widest chip
            at the same step the column narrows is what closes it, and `Type`
            keeps the slot because node types are the facet the legend itself is
            about. */}
        {typeFacet('br-facet-core')}
        {predicateFacet('br-facet-extra')}
        {sourceFacet('br-facet-extra')}
        {statusFacet('br-facet-extra')}

        {/* 860–1059px: the three least-used fold away, keeping their counts. */}
        <CollapsedFacets
          label="More"
          className="br-facet-more"
          count={facets.predicates.size + facets.sources.size + facets.statuses.size}
        >
          {predicateFacet('', 'knowledge-graph-facet-predicate-in-menu')}
          {sourceFacet('', 'knowledge-graph-facet-source-in-menu')}
          {statusFacet('', 'knowledge-graph-facet-status-in-menu')}
        </CollapsedFacets>

        {/* Below 860px: one control, and the search takes the rest of the row. */}
        <CollapsedFacets
          label="Filters"
          className="br-facet-all"
          count={
            facets.types.size + facets.predicates.size + facets.sources.size + facets.statuses.size
          }
        >
          {/* ⚠ **`-in-all`, not `-in-menu`, for all four.** Both popovers are in
              the DOM at every step — the ladder only toggles `display` — so a
              suffix shared with the `More` menu means two elements answer to one
              test id and `getByTestId` throws on a query that looks correct.
              `predicate` had exactly that collision the moment it joined `More`.
              The suffix names the popover, so every child of this one takes it. */}
          {typeFacet('', 'knowledge-graph-facet-type-in-all')}
          {predicateFacet('', 'knowledge-graph-facet-predicate-in-all')}
          {sourceFacet('', 'knowledge-graph-facet-source-in-all')}
          {statusFacet('', 'knowledge-graph-facet-status-in-all')}
        </CollapsedFacets>

        {/* ⚠ **The legend NEVER overlaps the canvas** (R-03). Below the widest
            step there is no room for a rail, and an earlier revision floated a
            card over the graph instead — measured at a 946px pane it covered
            44% of the canvas and sat on the nodes, which is the overlap
            complaint this redesign exists to fix, reintroduced by the fix. A
            popover shows the same legend on demand, covers nothing when closed,
            and is dismissible by construction. It retires once the rail is
            permanent. */}
        <LegendPopover model={model} mode={mode} facets={facets} onChange={onChange} />
      </div>

      {active && (
        <div className="flex flex-none items-center gap-2">
          {/* The count yields before the control that undoes the filter does:
              knowing HOW MANY are hidden is useful, being able to get them back
              is essential. */}
          <span className="br-knowledge-readout whitespace-nowrap text-supporting tabular-nums text-text-muted">
            {knowledgeCopy.graph.showing(passing, total)}
          </span>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-label={knowledgeCopy.graph.clearFilters}
                onClick={() =>
                  onChange({
                    search: '',
                    types: new Set(),
                    predicates: new Set(),
                    sources: new Set(),
                    statuses: new Set(),
                  })
                }
              >
                <X aria-hidden="true" />
                <span className="br-knowledge-readout">{knowledgeCopy.graph.clearFilters}</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">
              {knowledgeCopy.graph.clearFiltersHint(passing, total)}
            </TooltipContent>
          </Tooltip>
        </div>
      )}
    </div>
  );
}

/** The legend, reachable from the filter bar wherever the rail has no room. */
function LegendPopover({
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
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="br-facet br-facet-legend biorouter-focus-surface ml-auto inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color]"
          aria-expanded={open}
          data-testid="knowledge-graph-legend-trigger"
        >
          Legend
          <ChevronDown
            aria-hidden="true"
            className={`h-icon-row w-icon-row shrink-0 transition-transform ${open ? 'rotate-180' : ''}`}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-0">
        <GraphLegend
          variant="popover"
          model={model}
          mode={mode}
          facets={facets}
          onChange={onChange}
        />
      </PopoverContent>
    </Popover>
  );
}

/**
 * `Status` — the one facet with a fixed vocabulary, so it needs no search box
 * and takes a plain menu rather than the searchable picker.
 *
 * Extracted so it can be rendered inline in the row AND inside the collapsed
 * `More`/`Filters` menu from one definition (R-09).
 */
function StatusFacet({
  facets,
  onChange,
  extraClass = '',
  testId,
}: {
  facets: FacetState;
  onChange: (next: FacetState) => void;
  extraClass?: string;
  testId?: string;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className={`br-facet biorouter-focus-surface inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color] ${extraClass}`}
          data-engaged={facets.statuses.size > 0 ? 'true' : 'false'}
          data-testid={testId ?? 'knowledge-graph-facet-status'}
        >
          Status
          {facets.statuses.size > 0 && (
            <span className="rounded-inner bg-background-inverse/15 px-1.5 text-supporting tabular-nums">
              {facets.statuses.size}
            </span>
          )}
          <ChevronDown aria-hidden="true" className="h-icon-row w-icon-row shrink-0" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {STATUSES.map((st) => (
          <DropdownMenuCheckboxItem
            key={st}
            checked={facets.statuses.has(st)}
            onCheckedChange={() => onChange({ ...facets, statuses: toggle(facets.statuses, st) })}
          >
            {st}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The collapsed form of two or more facets (R-09).
 *
 * ⚠ **It renders the REAL pickers, not a copy of them.** The alternative — a
 * bespoke list reproducing the counts, the family grouping and the
 * multi-select — is the kind of second implementation that drifts from the
 * first the moment a facet gains a field. Nesting the same `FacetCombobox`
 * inside a popover costs one Radix layer and keeps exactly one picker in the
 * codebase.
 *
 * The count on the trigger is the SUM of what folded away, so a filter the user
 * cannot see is still reported by the control that swallowed it.
 */
function CollapsedFacets({
  label,
  className,
  count,
  children,
}: {
  label: string;
  className: string;
  count: number;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={`br-facet biorouter-focus-surface inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color] ${className}`}
          data-engaged={count > 0 ? 'true' : 'false'}
          aria-expanded={open}
          data-testid={`knowledge-graph-facet-${label.toLowerCase()}-collapsed`}
        >
          {label}
          {count > 0 && (
            <span className="rounded-inner bg-background-inverse/15 px-1.5 text-supporting tabular-nums">
              {count}
            </span>
          )}
          <ChevronDown
            aria-hidden="true"
            className={`h-icon-row w-icon-row shrink-0 transition-transform ${open ? 'rotate-180' : ''}`}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="flex w-56 flex-col items-stretch gap-1 p-2">
        {children}
      </PopoverContent>
    </Popover>
  );
}

interface Option {
  value: string;
  label: string;
  count: number;
  fill?: string;
  /** Provenance & Context draws as an open ring, matching the canvas mark. */
  hollow?: boolean;
  negated?: boolean;
}

/**
 * One facet button and its searchable list (§4.0's one picker pattern).
 *
 * The rows carry the palette swatch INSIDE its family's shape glyph, because
 * that is the mapping the canvas uses and a facet row is where a user is most
 * likely to be learning it.
 */
function FacetCombobox({
  label,
  options,
  selected,
  onToggle,
  mode,
  extraClass = '',
  testId,
}: {
  label: string;
  options: Option[];
  selected: Set<string>;
  onToggle: (value: string) => void;
  mode: GraphMode;
  /** Which step of R-09's ladder this facet belongs to (`.br-facet-core` etc). */
  extraClass?: string;
  /**
   * Overridden when the same facet is rendered a second time inside the
   * collapsed `More`/`Filters` menu, so the two instances never share a test id
   * — `getByTestId` throws on a duplicate, and a silently ambiguous query is
   * worse than a loud one.
   */
  testId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');

  const families = GRAPH_PALETTE[mode].families;
  const familyOf = useMemo(() => {
    const map = new Map<string, string>();
    for (const [name, family] of Object.entries(families)) {
      for (const member of family.members) map.set(member, name);
    }
    return map;
  }, [families]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return needle ? options.filter((o) => o.label.toLowerCase().includes(needle)) : options;
  }, [options, query]);

  /**
   * BioOKF groups by family; OKF is flat and sorted by count.
   *
   * The test is whether any option is a member of a curated family at all — in
   * an OKF base every type takes the hashed fallback and the circle, so there
   * are no families to group by and a heading per type would be noise.
   */
  const grouped = useMemo(() => {
    if (label !== 'Type') return null;
    // Seeded in the palette's own FAMILY ORDER, not in encounter order. The
    // ladder — Genomic → Molecular → Anatomy → Clinical → Exposome → Physical →
    // Provenance — is the order the legend teaches and the order the fills
    // ramp in; sorting by whichever family happened to have the commonest type
    // would put a different family first for every base the user opens.
    const groups = new Map<string, Option[]>(Object.keys(families).map((name) => [name, []]));
    let anyFamily = false;
    for (const o of filtered) {
      const family = familyOf.get(o.value);
      if (family) anyFamily = true;
      const key = family ?? 'Other types';
      const bucket = groups.get(key);
      if (bucket) bucket.push(o);
      else groups.set(key, [o]);
    }
    for (const [name, rows] of groups) if (rows.length === 0) groups.delete(name);
    return anyFamily ? groups : null;
  }, [families, filtered, familyOf, label]);

  const row = (o: Option) => (
    <CommandItem key={o.value} selected={selected.has(o.value)} onSelect={() => onToggle(o.value)}>
      {o.fill ? (
        <NodeSwatch fill={o.fill} hollow={o.hollow} className="br-swatch-ring" />
      ) : (
        <span aria-hidden="true" className="h-3 w-3 flex-none" />
      )}
      <span
        className={['min-w-0 flex-1 truncate', o.negated ? 'text-text-danger line-through' : '']
          .filter(Boolean)
          .join(' ')}
      >
        {o.label}
      </span>
      {o.count > 0 && (
        <span className="flex-none text-supporting tabular-nums text-text-muted">{o.count}</span>
      )}
    </CommandItem>
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {/* ⚠ **NOT a `Button`, and that is the fix** (R-02). These were
            `<Button variant="secondary" size="sm">` — the same component and
            variant as `Manage bases` — sharing the search `Input`'s exact 8px
            radius, so a field and a toggle were indistinguishable in the same
            row. A filter is differentiated by EDGE AND GROUND, not by shape: it
            keeps `--radius-element` like every button, input and tab in the
            app, and takes a 1.5px border on a transparent fill so it reads as a
            field that holds a value where a secondary button is a filled slab
            with no edge at all. Engaged is a solid accent fill rather than a
            tint — across a 1600px bar a tint is a guess.

            An earlier revision made these pills (`--radius-full`) and a later
            one made them 4px; both introduced a shape the app uses nowhere
            else. `.br-facet` in main.css carries the resting/engaged pair. */}
        <button
          type="button"
          className={`br-facet biorouter-focus-surface inline-flex h-control-md flex-none items-center gap-2 rounded-element px-2.5 text-label transition-[color,background-color,border-color] ${extraClass}`}
          data-engaged={selected.size > 0 ? 'true' : 'false'}
          aria-expanded={open}
          data-testid={testId ?? `knowledge-graph-facet-${label.toLowerCase()}`}
        >
          {label}
          {selected.size > 0 && (
            <span className="rounded-inner bg-background-inverse/15 px-1.5 text-supporting tabular-nums">
              {selected.size}
            </span>
          )}
          <ChevronDown
            aria-hidden="true"
            className={`h-icon-row w-icon-row shrink-0 transition-transform ${open ? 'rotate-180' : ''}`}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-0">
        <Command
          label={`${label} filter`}
          query={query}
          onQueryChange={setQuery}
          className="flex max-h-72 flex-col"
        >
          <CommandInput placeholder={`Search ${label.toLowerCase()}s`} />
          <CommandList>
            {grouped
              ? [...grouped.entries()].map(([family, rows]) => (
                  <CommandGroup
                    key={family}
                    heading={<span className="flex items-center gap-2">{family}</span>}
                  >
                    {rows.map(row)}
                  </CommandGroup>
                ))
              : filtered.map(row)}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
