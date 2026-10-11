// ui/desktop/src/components/knowledge/graph/KnowledgeGraphPanel.tsx
import { useMemo, useState } from 'react';
import { AlertCircle, Sparkles } from '../../icons/app-icons';
import type { Graph, GraphEdge, GraphNode } from '../../../api/types.gen';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { Spinner } from '../../ui/spinner';
import { useResolvedTheme } from '../../../contexts/ThemeContext';
import { ForceGraphCanvas } from './ForceGraphCanvas';
import { GraphFacetStrip } from './GraphFacetStrip';
import { GraphLegend } from './GraphLegend';
import { EdgePreview } from './EdgePreview';
import { NodePreview } from './NodePreview';
import { buildGraphModel } from './graphModel';
import { applyFacets, EMPTY_FACETS, facetsActive } from './graphFacets';
import type { FacetState } from './graphFacets';

interface Props {
  kbId: string | null;
  graph: Graph | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  /// When set, the panel is in read-only "preview at SHA" mode. The banner
  /// shows the SHA; the graph still renders the current data (ghosting of
  /// future-state nodes is a Plan-6 polish item).
  previewSha: string | null;
  onClearPreview: () => void;
}

/**
 * The graph pane (ui-spec §4.5, amended by redesign R-03).
 *
 * ⚠ **THE BOTTOM LEGEND DOCK IS GONE.** It was a real flex sibling, so it did
 * not float over the canvas — it TOOK 36px from it, permanently, to show a row
 * that scrolled sideways and was entirely inert. The legend is now the right
 * rail at the widest step and a dismissible canvas-anchored card below it, both
 * rendered by one `GraphLegend`; the container query in `main.css` decides
 * which. The canvas keeps the height the dock used to spend.
 *
 * ⚠ **The model is built HERE, once per graph, and passed down.** All three
 * regions read the same derived counts, and DR-9 is explicit that the label
 * pass and the radius model must not be paid `nodes × 60` times a second. A
 * canvas that derived its own would also disagree with the legend that claims
 * to describe it.
 *
 * ⚠ **Two loading behaviours, and conflating them is the bug this fixes.** The
 * FIRST load of a base's graph cross-fades a `role="status"` block against the
 * canvas. A REFRESH of a graph already on screen does **not** blank it: the
 * Refresh button's icon spins and nothing else changes. Blanking a graph the
 * user is reading, in order to redraw the same graph, is a regression disguised
 * as feedback.
 *
 * ⚠ **A flush column, not a card** (spec 3.12). The pane used to be a bordered,
 * rounded box beside the Sources card; the workspace now fills edge to edge
 * under the band and the columns divide by Crew's inset hairline.
 */
export function KnowledgeGraphPanel({
  kbId,
  graph,
  loading,
  error,
  onRefresh,
  previewSha,
  onClearPreview,
}: Props) {
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [selected, setSelected] = useState<GraphNode | null>(null);
  // ⚠ **One inspector at a time.** Both panels dock to the same top-right
  // corner, so two open at once is one panel with another on top of it —
  // selecting either therefore clears the other rather than stacking.
  const [selectedEdge, setSelectedEdge] = useState<GraphEdge | null>(null);
  const [facets, setFacets] = useState<FacetState>(EMPTY_FACETS);
  const mode = useResolvedTheme();

  const firstLoad = loading && !graph;
  const hasNodes = !!graph && graph.nodes.length > 0;

  const model = useMemo(() => (graph ? buildGraphModel(graph) : null), [graph]);
  // The edge inspector draws each endpoint's real mark, so it needs the NODE
  // rather than the model's `NodeMetrics` (which carries `type` but no `kind`).
  // Built here, once per graph, for the same reason the model is.
  const nodesById = useMemo(() => new Map((graph?.nodes ?? []).map((n) => [n.id, n])), [graph]);
  const facetResult = useMemo(() => (graph ? applyFacets(graph, facets) : null), [graph, facets]);
  const active = facetsActive(facets);

  return (
    <div className="relative flex h-full w-full min-w-0 flex-1 flex-col overflow-hidden bg-background-default">
      {previewSha && (
        <div className="flex flex-none items-center justify-between gap-2 border-b border-border-subtle bg-wash-warning px-3 py-2 text-supporting text-text-warning">
          <span>Previewing commit {previewSha.slice(0, 7)}: read-only</span>
          <Button variant="ghost" size="sm" onClick={onClearPreview}>
            Exit preview
          </Button>
        </div>
      )}

      {hasNodes && model && facetResult && (
        <GraphFacetStrip
          model={model}
          mode={mode}
          facets={facets}
          onChange={setFacets}
          passing={facetResult.passing}
          total={facetResult.total}
          active={active}
        />
      )}

      {/* canvas + detail rail. The rail only exists at the widest step; below
          it the same legend renders as the dismissible card inside the canvas. */}
      <div className="flex min-h-0 flex-1">
        <div className="br-knowledge-recessed relative min-h-0 min-w-0 flex-1 overflow-hidden bg-background-muted">
          {/* Two absolutely-stacked opacity layers, never a swap: loading out at
            `--dur-fast`, content in at `--dur-med`. */}
          <div
            role="status"
            className={`br-knowledge-fade-out absolute inset-0 flex items-center justify-center gap-2 ${
              firstLoad ? 'opacity-100' : 'pointer-events-none opacity-0'
            }`}
          >
            {firstLoad && (
              <>
                <Spinner className="text-text-muted" />
                <span className="text-secondary text-text-muted">Loading graph</span>
              </>
            )}
          </div>

          {/* `flex` so an `EmptyState` (which centres itself only within its own
            box) is centred in the CANVAS, and so the canvas child still gets the
            full height it measures against. */}
          <div
            className={`br-knowledge-fade-in absolute inset-0 flex ${
              firstLoad ? 'opacity-0' : 'opacity-100'
            }`}
          >
            {error ? (
              <EmptyState
                className="m-auto"
                icon={AlertCircle}
                title="Could not load the graph"
                description={error}
                actions={
                  // The canvas ground is `--background-muted`, the very value the resting fill
                  // has (1.00:1), so a filled control here rests on the strong step and hovers one
                  // fill deeper: `.br-knowledge-recessed` on the canvas sets both for every filled
                  // control on it (R-27, design.md 9.11), so the button carries no ground of its own.
                  <Button variant="secondary" onClick={onRefresh}>
                    Try again
                  </Button>
                }
              />
            ) : graph && !hasNodes ? (
              <EmptyState
                className="m-auto"
                icon={Sparkles}
                title="Nothing digested yet"
                description="Stage a source in the Sources rail and press Digest."
              />
            ) : hasNodes && model ? (
              <ForceGraphCanvas
                graph={graph}
                model={model}
                facets={facets}
                passing={facetResult?.nodes ?? null}
                selectedId={selected?.id ?? null}
                hoveredId={hoveredId}
                onHover={setHoveredId}
                onNodeClick={(n) => {
                  setSelectedEdge(null);
                  setSelected(n);
                }}
                onLinkClick={(e) => {
                  setSelected(null);
                  setSelectedEdge(e);
                }}
                visibleSet={null}
              />
            ) : null}
          </div>

          {selected && kbId && (
            <NodePreview
              kbId={kbId}
              node={selected}
              mode={mode}
              previewSha={previewSha}
              onClose={() => setSelected(null)}
            />
          )}

          {selectedEdge && model && (
            <EdgePreview
              edge={selectedEdge}
              model={model}
              nodeById={(id) => nodesById.get(id)}
              mode={mode}
              onSelectNode={(n) => {
                setSelectedEdge(null);
                setSelected(n);
              }}
              onClose={() => setSelectedEdge(null)}
            />
          )}
        </div>

        {hasNodes && model && (
          <GraphLegend
            variant="rail"
            model={model}
            mode={mode}
            facets={facets}
            onChange={setFacets}
          />
        )}
      </div>
    </div>
  );
}
