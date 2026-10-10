// ui/desktop/src/components/knowledge/graph/EdgePreview.tsx
import { useEffect, useRef } from 'react';
import { ArrowDown } from '../../icons/app-icons';
import type { GraphEdge, GraphNode, QuantitativeValue } from '../../../api/types.gen';
import type { GraphMode } from '../../../styles/graphPalette';
import { Badge } from '../../ui/badge';
import { InfoTip } from '../../ui/info-tip';
import { knowledgeCopy } from '../copy';
import { KnowledgePaneHeader } from '../KnowledgePaneHeader';
import { fillFor, isHollow } from './nodeMark';
import { NodeSwatch } from './NodeSwatch';
import { xrefHref } from './frontmatter';
import { isNegated, readablePredicate } from './graphModel';
import type { GraphModel } from './graphModel';

/**
 * The edge inspector (ui-spec §4.8, "Edge inspector — body order").
 *
 * ⚠ **The canvas already tracked a hovered edge and painted its label; nothing
 * could OPEN one.** `onLinkClick` was unwired, so every field an edge carries
 * beyond its predicate — the §8.1 provenance triplet, the §7.3 quantitative
 * bundle, the §7.2 qualifiers — was derived by the daemon, serialized over the
 * wire, and unreachable. A user could see that Metformin treats Diabetes and
 * had no way to ask on whose authority, from which source, at what effect size.
 *
 * ⚠ **A missing provenance field is SHOWN as missing.** "Not stated" is the
 * answer to "who says so"; omitting the row would make an unsourced claim look
 * the same as a sourced one, which is the single most consequential confusion
 * this panel can produce.
 *
 * ⚠ **A negated edge is a REFUTED claim, not an absent one**, and the panel
 * spends three channels saying so rather than trusting one: a `danger`-toned
 * "Negative edge" badge in the head, the word `not` spelled into the predicate,
 * and the strike-through. §5.8's argument for word-level redundancy on the
 * canvas applies with more force here — this is the surface a user opens
 * precisely *because* they are about to rely on the claim.
 */

interface Props {
  edge: GraphEdge;
  /** For resolving endpoint ids to the labels the canvas draws. */
  model: GraphModel;
  /**
   * The endpoint nodes themselves, so the headline can draw each one's real
   * mark. `NodeMetrics` carries a `type` but no `kind`, and `fillFor` needs the
   * node — synthesising a stand-in from `type` alone would be a second
   * derivation of the mark, which is the exact bug `nodeMark.ts` exists to shut.
   */
  nodeById: (id: string) => GraphNode | undefined;
  mode: GraphMode;
  /** Selecting an endpoint or the primary source moves the inspector to it. */
  onSelectNode?: (node: GraphNode) => void;
  onClose: () => void;
}

/** The three §8.1 fields, in the order the spec lists them. */
const PROVENANCE: { key: 'knowledge_level' | 'agent_type' | 'primary_source'; label: string }[] = [
  { key: 'knowledge_level', label: 'Knowledge level' },
  { key: 'agent_type', label: 'Agent type' },
  { key: 'primary_source', label: 'Primary source' },
];

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="br-inspector-row">
      <span className="text-supporting text-text-muted">{label}</span>
      <span className="min-w-0 break-words text-body text-text-default">{children}</span>
    </div>
  );
}

/**
 * §4.8 item 5's one privileged merge: a confidence interval is ONE fact.
 *
 * `ci_lower: 1.2` and `ci_upper: 3.4` on separate rows invites the reader to
 * treat the bounds as independent quantities; nobody reports half an interval.
 * Everything else in `quantitative` and `qualifiers` is rendered uniformly, so a
 * vocabulary addition still shows up with no code change here.
 */
export function mergeQuantitative(
  quantitative: Record<string, QuantitativeValue> | undefined
): [string, string][] {
  const entries = Object.entries(quantitative ?? {});
  const lower = quantitative?.ci_lower;
  const upper = quantitative?.ci_upper;
  const merged: [string, string][] = [];

  for (const [key, value] of entries) {
    if (key === 'ci_lower' || key === 'ci_upper') continue;
    merged.push([key.replace(/_/g, ' '), String(value)]);
  }

  if (lower != null && upper != null) {
    merged.push(['95% CI', knowledgeCopy.graph.ciRange(lower, upper)]);
  } else if (lower != null) {
    merged.push(['95% CI lower', String(lower)]);
  } else if (upper != null) {
    merged.push(['95% CI upper', String(upper)]);
  }

  return merged;
}

export function EdgePreview({ edge, model, nodeById, mode, onSelectNode, onClose }: Props) {
  const panelRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    const handlePointerDown = (event: globalThis.PointerEvent) => {
      if (!panelRef.current?.contains(event.target as Node)) onCloseRef.current();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onCloseRef.current();
    };

    document.addEventListener('pointerdown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  const label = (id: string) => model.nodes.get(id)?.display ?? id;
  const predicate = readablePredicate(edge);
  const negated = isNegated(edge);
  const quantitative = mergeQuantitative(edge.quantitative);
  const qualifiers = Object.entries(edge.qualifiers ?? {});

  /** One endpoint row of the headline: the node's own mark, then its identifier. */
  const Endpoint = ({ id }: { id: string }) => {
    const node = nodeById(id);
    const body = (
      <>
        {node ? (
          <NodeSwatch
            fill={fillFor(node, mode)}
            hollow={isHollow(node, mode)}
            size={8}
            className="br-swatch-ring flex-none"
          />
        ) : (
          // An endpoint outside this bundle has no node and therefore no mark.
          // Drawing a default one would assert a type it does not have.
          <span
            aria-hidden="true"
            className="h-2 w-2 flex-none rounded-full bg-background-medium"
          />
        )}
        <span className="min-w-0 truncate text-body">{label(id)}</span>
      </>
    );

    if (!node || !onSelectNode) {
      return <span className="flex min-w-0 items-center gap-2 py-1">{body}</span>;
    }
    return (
      <button
        type="button"
        onClick={() => onSelectNode(node)}
        className="flex min-w-0 items-center gap-2 rounded-inner py-1 text-left"
      >
        {body}
      </button>
    );
  };

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={`Link ${label(edge.from)} ${predicate} ${label(edge.to)}`}
      data-testid="knowledge-edge-preview"
      className="absolute top-12 right-4 z-[var(--z-dropdown)] flex max-h-[calc(100%-5rem)] w-[min(360px,calc(100%-2rem))] flex-col overflow-hidden rounded-container border border-border-subtle bg-background-default shadow-popover"
    >
      {/* 1. Head: Crew's pane header (spec 3.12), one 44px line. A negative
          edge says so in the title's ink; how the edge came about leads the
          body. */}
      <KnowledgePaneHeader
        title={
          <h2
            className={`min-w-0 truncate text-label ${negated ? 'text-text-danger' : 'text-text-default'}`}
          >
            {negated ? knowledgeCopy.graph.negativeEdge : knowledgeCopy.graph.edge}
          </h2>
        }
        onClose={onClose}
        closeLabel={knowledgeCopy.graph.closeEdge}
      />

      <div className="flex-1 overflow-y-auto px-4 py-3">
        <div className="flex flex-col gap-4">
          {/* 2. Headline — the edge as a sentence, STACKED. At 340px a single
              line truncates both endpoints, and the two endpoints are the part
              of an edge a reader cannot infer from anything else on screen. */}
          <span className="text-supporting text-text-muted">
            {edge.synthesized
              ? knowledgeCopy.graph.synthesizedFrom(edge.primary_source)
              : knowledgeCopy.graph.directed}
          </span>

          <section aria-label="Claim" className="flex flex-col">
            <Endpoint id={edge.from} />
            <div className="flex items-center gap-2 py-1">
              <ArrowDown
                aria-hidden="true"
                className="h-icon-chip w-icon-chip flex-none text-text-muted"
              />
              <Badge
                variant="chip"
                tone={negated ? 'danger' : 'neutral'}
                className={negated ? 'line-through' : undefined}
              >
                {predicate}
              </Badge>
            </div>
            <Endpoint id={edge.to} />
          </section>

          {/* 3. Provenance triplet. */}
          <section aria-label="Provenance" className="flex flex-col">
            <h3 className="text-caps text-text-muted">Provenance</h3>
            {edge.synthesized ? (
              // §4.8 replaces the triplet outright for a derived edge: it has no
              // author, so reporting three "not stated" rows would invite the
              // reader to go looking for an authority that cannot exist.
              <p className="flex items-center gap-1 py-2 text-body text-text-default">
                {knowledgeCopy.graph.derivedNote}
                <InfoTip
                  label={knowledgeCopy.graph.derivedNoteLabel}
                  help={knowledgeCopy.graph.derivedNoteInfo}
                />
              </p>
            ) : (
              PROVENANCE.map(({ key, label: rowLabel }) => (
                <Row key={key} label={rowLabel}>
                  {edge[key] ? (
                    <span className="font-mono text-code">{edge[key]}</span>
                  ) : (
                    <span className="text-text-muted">Not stated</span>
                  )}
                </Row>
              ))
            )}
          </section>

          {/* 4. Publications — real external links. */}
          {(edge.publications?.length ?? 0) > 0 && (
            <section aria-label="Publications" className="flex flex-col">
              <h3 className="text-caps text-text-muted">Publications</h3>
              <div className="flex flex-wrap gap-1 py-2">
                {edge.publications?.map((p) => {
                  const href = xrefHref(p);
                  return href ? (
                    <Badge key={p} variant="chip">
                      <a
                        href={href}
                        title={href}
                        onClick={(event) => {
                          event.preventDefault();
                          void window.electron?.openExternal(href);
                        }}
                        className="font-mono text-code text-text-accent underline underline-offset-2"
                      >
                        {p}
                      </a>
                    </Badge>
                  ) : (
                    <Badge key={p} variant="chip" className="font-mono">
                      {p}
                    </Badge>
                  );
                })}
              </div>
            </section>
          )}

          {/* 5. Stats and qualifiers, rendered uniformly. */}
          {quantitative.length > 0 && (
            <section aria-label="Quantitative" className="flex flex-col">
              <h3 className="text-caps text-text-muted">Quantitative</h3>
              {quantitative.map(([key, value]) => (
                <Row key={key} label={key}>
                  {/* Figures in the sans face with tabular figures (principle 3):
                      mono is for machine strings, and a p-value is not one. */}
                  <span className="tabular-nums">{value}</span>
                </Row>
              ))}
            </section>
          )}

          {qualifiers.length > 0 && (
            <section aria-label="Context" className="flex flex-col">
              <h3 className="text-caps text-text-muted">Context</h3>
              {qualifiers.map(([key, value]) => (
                <Row key={key} label={key.replace(/_/g, ' ')}>
                  <span>{value}</span>
                </Row>
              ))}
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
