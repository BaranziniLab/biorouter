// ui/desktop/src/components/knowledge/graph/NodePreview.tsx
import { useEffect, useMemo, useRef } from 'react';
import { FileCode2 } from '../../icons/app-icons';
import type { GraphNode } from '../../../api/types.gen';
import type { GraphMode } from '../../../styles/graphPalette';
import MarkdownContent from '../../MarkdownContent';
import { Badge } from '../../ui/badge';
import type { BadgeTone } from '../../ui/badge';
import { KnowledgePaneHeader } from '../KnowledgePaneHeader';
import { usePagePreview } from '../hooks/usePagePreview';
import { CredibilityRing, TIER_LABEL } from './CredibilityRing';
import { credibilityKey, fillFor, isHollow } from './nodeMark';
import { NodeSwatch } from './NodeSwatch';
import { frontmatterRows, splitFrontmatter, xrefHref } from './frontmatter';
import type { FrontmatterRow } from './frontmatter';
import { prettyLabel } from './labelText';

interface Props {
  kbId: string;
  node: GraphNode;
  /**
   * The resolved light/dark mode, passed in rather than read here so the
   * inspector and the canvas it describes resolve the palette from the same
   * value on the same commit — the facet strip and the legend take it the same
   * way, from `KnowledgeGraphPanel`.
   */
  mode: GraphMode;
  previewSha?: string | null;
  onClose: () => void;
}

/**
 * The node's display identity (ui-spec §4.8 item 1).
 *
 * `identifier` first, `label` second — the same order `buildGraphModel` uses for
 * the canvas label. The two used to disagree: the inspector's title showed the
 * slug (`metformin`) while the frontmatter block one line below it showed the
 * `identifier` (`Metformin`), so the panel contradicted itself inside 40px.
 */
export function nodeTitle(node: GraphNode): string {
  return prettyLabel(node.identifier || node.label, node.kind);
}

/**
 * One identity fact — a `tone` when it is a badge, `null` when it is plain ink.
 *
 * ⚠ **The badges and the text summary are derived from THIS, not from each
 * other and not from the node twice.** Two renderings of one fact set, written
 * separately, is the shape of the bug this whole pass exists to close; it does
 * not stop being that shape because both renderings happen to live in one file.
 */
export interface NodeFact {
  key: string;
  text: string;
  tone: BadgeTone | null;
  strike?: boolean;
}

/**
 * §4.8 item 1's sub-line, as data.
 *
 * ⚠ **It reads `node_type`, and only falls back to `kind` when there is none.**
 * The deriver writes `kind: 'hub'` onto every typed page, so a subtitle written
 * against `kind` said "hub" for every node in an OKF base — beside a canvas that
 * was correctly drawing the same node as a `Drug`. An untyped page is labelled
 * `Untyped` and then by its legacy `kind`, because DR-28 makes absence a real
 * state that has to be *shown*, not filled in.
 *
 * ⚠ **`stable` deliberately emits nothing.** §4.8 gives `draft` a warning tone
 * and `deprecated` a struck neutral one, and gives the ordinary case no badge at
 * all: a panel that decorates the normal state spends the reader's attention on
 * the 90% of pages that need none, and leaves the two that matter competing with
 * it. Any *other* status still renders, as a neutral badge — there is no
 * allowlist, so a vocabulary addition shows up without a code change.
 */
export function nodeFacts(node: GraphNode, previewSha?: string | null): NodeFact[] {
  const facts: NodeFact[] = [];

  if (node.node_type) {
    facts.push({ key: 'type', text: node.node_type, tone: 'neutral' });
  } else {
    // DR-28: a legacy page genuinely has no type. Name the absence and then the
    // only identity such a page has, rather than inventing a type for it.
    facts.push({ key: 'type', text: 'Untyped', tone: 'neutral' });
    facts.push({ key: 'kind', text: node.kind, tone: null });
  }

  if (node.subtype) facts.push({ key: 'subtype', text: node.subtype, tone: null });

  if (node.status && node.status !== 'stable') {
    facts.push({
      key: 'status',
      text: node.status,
      tone: node.status === 'draft' ? 'warning' : 'neutral',
      strike: node.status === 'deprecated',
    });
  }

  if (node.stale) facts.push({ key: 'stale', text: 'Stale', tone: 'warning' });
  if (previewSha) facts.push({ key: 'sha', text: previewSha.slice(0, 7), tone: null });

  return facts;
}

/** The same facts as one line of text — the panel's accessible summary. */
export function nodeSubtitle(node: GraphNode, previewSha?: string | null): string {
  return nodeFacts(node, previewSha)
    .map((f) => f.text)
    .join(' · ');
}

/** An `xref` token that resolved to a real identifier, opened in the OS browser. */
function ExternalToken({ text, href }: { text: string; href: string }) {
  return (
    <a
      href={href}
      title={href}
      onClick={(event) => {
        // The renderer must never navigate itself to an external origin.
        event.preventDefault();
        void window.electron?.openExternal(href);
      }}
      className="font-mono text-code text-text-accent underline underline-offset-2"
    >
      {text}
    </a>
  );
}

/** One frontmatter key and its value, in whichever of the three shapes it takes. */
function FrontmatterRowView({ row }: { row: FrontmatterRow }) {
  return (
    <div className="br-inspector-row">
      <span className="text-caps text-text-muted">{row.label}</span>
      <div className="min-w-0 text-body text-text-default">
        {row.value.kind === 'text' && <span className="break-words">{row.value.text}</span>}

        {row.value.kind === 'chips' && (
          <span className="flex flex-wrap gap-1">
            {row.value.items.map((item, i) =>
              item.href ? (
                <Badge key={`${item.text}-${i}`} variant="chip">
                  <ExternalToken text={item.text} href={item.href} />
                </Badge>
              ) : (
                <Badge key={`${item.text}-${i}`} variant="chip">
                  {item.text}
                </Badge>
              )
            )}
          </span>
        )}

        {row.value.kind === 'entries' && (
          <div className="flex flex-col gap-2">
            {row.value.entries.map((entry, i) => (
              <div key={i} className="flex flex-col gap-0.5">
                {entry.map((field) => (
                  <div key={field.label} className="flex gap-2">
                    <span className="flex-none text-supporting text-text-muted">{field.label}</span>
                    <span className="min-w-0 break-words text-supporting">
                      {xrefOrText(field.text)}
                    </span>
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * A `sources[]` field value, linked when it happens to be a resolvable token.
 *
 * Through the SAME resolver the chips use, so a `resource: DOI:10.x` inside a
 * source entry links exactly as that identical token does in an `xref` array.
 * One resolver, every position.
 */
function xrefOrText(text: string) {
  const href = xrefHref(text);
  return href ? <ExternalToken text={text} href={href} /> : text;
}

export function NodePreview({ kbId, node, mode, previewSha, onClose }: Props) {
  const { content, loading, error } = usePagePreview(kbId, node.path, previewSha);
  const parsed = useMemo(() => splitFrontmatter(content ?? ''), [content]);
  const rows = useMemo(
    () => (parsed.frontmatter ? frontmatterRows(parsed.frontmatter) : null),
    [parsed.frontmatter]
  );
  const facts = nodeFacts(node, previewSha);
  const ringKey = credibilityKey(node);
  const showsProvenance = node.credibility_tier != null || node.retracted === true;
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

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={`Preview ${nodeTitle(node)}`}
      data-testid="knowledge-node-preview"
      className="absolute top-12 right-4 z-[var(--z-dropdown)] flex max-h-[calc(100%-5rem)] w-[min(360px,calc(100%-2rem))] flex-col overflow-hidden rounded-container border border-border-subtle bg-background-default shadow-popover"
    >
      {/* Crew's pane header (spec 3.12): one 44px line, the node's mark and its
          name, and the close. The facts that shared the old two-line header
          lead the body instead. */}
      <KnowledgePaneHeader
        leading={
          // The SAME mark the canvas paints, from the same functions — fill by
          // `node_type` through `GRAPH_PALETTE`, solid or hollow by family. A
          // swatch that disagrees with the mark it describes is worse than none.
          <NodeSwatch
            fill={fillFor(node, mode)}
            hollow={isHollow(node, mode)}
            size={12}
            className="br-swatch-ring flex-shrink-0"
          />
        }
        title={<h2 className="min-w-0 truncate text-label">{nodeTitle(node)}</h2>}
        onClose={onClose}
        closeLabel="Close preview"
      />

      <div className="flex-1 overflow-y-auto px-4 py-3 text-body text-text-default">
        <div className="space-y-4">
          {facts.length > 0 && (
            <div data-testid="knowledge-node-facts" className="flex flex-wrap items-center gap-1">
              {facts.map((fact) =>
                fact.tone ? (
                  <Badge
                    key={fact.key}
                    tone={fact.tone}
                    className={fact.strike ? 'line-through' : undefined}
                  >
                    {fact.text}
                  </Badge>
                ) : (
                  <span key={fact.key} className="text-supporting text-text-muted">
                    {fact.text}
                  </span>
                )
              )}
            </div>
          )}
          {/* §4.8 item 3. Identity and provenance come off the graph node, which
              is already in memory, so they paint immediately and do not wait on
              the page fetch below. */}
          {showsProvenance && (
            <section aria-label="Provenance" className="flex flex-wrap items-center gap-2">
              {ringKey && <CredibilityRing tier={ringKey} mode={mode} />}
              {node.credibility_tier && (
                // ⚠ `tone="neutral"` — app ink on an app surface. The tier hue
                // stays in the ring and NEVER behind these words: the seven ring
                // colours are solved for a 1.6px arc against the graph ground,
                // and as a text background they are neither legible nor a
                // passing pair.
                <Badge tone="neutral">{TIER_LABEL[node.credibility_tier]}</Badge>
              )}
              {node.retracted && <Badge tone="danger">Retracted</Badge>}
            </section>
          )}

          {loading && <span className="text-text-muted">Loading page…</span>}
          {error && <span className="text-text-danger">{error}</span>}

          {!loading && !error && content && (
            <>
              {parsed.frontmatter &&
                // §4.8 item 2. Parsed rows when the block is a YAML mapping; the
                // raw text when it is not, because frontmatter that fails to
                // parse is still frontmatter the user may need to SEE to fix.
                (rows ? (
                  <div className="overflow-hidden rounded-element bg-background-muted px-3 py-1">
                    {rows.map((row) => (
                      <FrontmatterRowView key={row.key} row={row} />
                    ))}
                  </div>
                ) : (
                  <div className="overflow-hidden rounded-element bg-background-muted">
                    <div className="flex items-center gap-2 px-3 py-1.5 text-caps text-text-muted">
                      <FileCode2 className="h-3 w-3" />
                      Unparsed frontmatter
                    </div>
                    <pre className="overflow-x-auto px-3 py-2.5 font-mono text-code whitespace-pre-wrap text-text-muted">
                      {parsed.frontmatter}
                    </pre>
                  </div>
                ))}

              {parsed.body ? (
                <MarkdownContent content={parsed.body} className="text-body" />
              ) : (
                <span className="text-text-muted">No body content.</span>
              )}
            </>
          )}

          {!loading && !error && !content && <span className="text-text-muted">No content.</span>}
        </div>
      </div>

      <div className="border-t border-border-subtle bg-background-muted px-4 py-2 font-mono text-supporting break-all text-text-muted">
        {node.path}
      </div>
    </div>
  );
}
