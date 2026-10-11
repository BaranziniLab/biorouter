// ui/desktop/src/components/knowledge/lint/LintDrawer.tsx
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { AlertCircle, AlertTriangle, CheckCircle2, Info } from '../../icons/app-icons';
import type { Diagnostic, LintResult, Severity } from '../../../api/types.gen';
import { Badge } from '../../ui/badge';
import type { BadgeTone } from '../../ui/badge';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { InfoTip } from '../../ui/info-tip';
import { Sheet, SheetContent, SheetTitle } from '../../ui/sheet';
import { Spinner } from '../../ui/spinner';
import { knowledgeCopy } from '../copy';
import { KnowledgePaneHeader } from '../KnowledgePaneHeader';
import { useModelAndProvider } from '../../ModelAndProviderContext';
import { useKnowledge } from '../KnowledgeContext';
import { useIngestStream } from '../hooks/useIngestStream';
import { resolveIngestModel } from '../IngestPanel/resolveIngestModel';

/**
 * Running the base's lint, and reading what it found (ui-spec §4.11).
 *
 * ⚠ **The route and the generated client have always existed; nothing called
 * them.** `lint` is exported from `api/sdk.gen.ts` and `POST
 * /knowledge/bases/{id}/lint` answers, but no component imported it — so the
 * format chooser's promise that "validation flags anything outside the
 * vocabulary" was a promise with no surface behind it. A user could be told
 * their base had problems only by reading the daemon's log.
 *
 * ⚠ **The stream is `useIngestStream`, deliberately, not a second hook.** The
 * lint route is a POST that answers over SSE — the same shape the ingest route
 * has, down to the terminal `event: done` frame — and that hook already carries
 * the two things a second implementation would get wrong: `EventSource` cannot
 * POST, so it is raw `fetch` + `ReadableStream`; and a body that ends WITHOUT a
 * terminal frame is a FAILURE, never a silent success (issue #71). Its name says
 * ingest and its behaviour is "one macro stream".
 *
 * ⚠ **Read-only.** `autofix` is never sent, so the lint reports and changes
 * nothing. An autofix rewrites pages and commits, which is a different decision
 * with a different confirmation, and this surface exists to answer "what is
 * wrong" first.
 */

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * Severity order, and the one place it is written.
 *
 * Most severe first, so a base with one error among forty infos still opens on
 * the error. The daemon already sorts `items` this way; the grouping does not
 * rely on that, because a caller that re-sorts is not a contract.
 */
const SEVERITIES: Severity[] = ['error', 'warning', 'info'];

const SEVERITY_TONE: Record<Severity, BadgeTone> = {
  error: 'danger',
  warning: 'warning',
  info: 'info',
};

const SEVERITY_ICON: Record<Severity, typeof AlertCircle> = {
  error: AlertCircle,
  warning: AlertTriangle,
  info: Info,
};

const SEVERITY_LABEL: Record<Severity, string> = {
  error: 'Errors',
  warning: 'Warnings',
  info: 'Notes',
};

/** The four hygiene lists the report carries beside its typed diagnostics. */
const REPORT_LISTS: { key: keyof LintReportLists; label: string; hint: string }[] = [
  {
    key: 'contradictions',
    label: 'Contradictions',
    hint: 'Pages whose frontmatter marks them as contradicting another page.',
  },
  {
    key: 'orphans',
    label: 'Orphans',
    hint: 'Pages nothing else links to.',
  },
  {
    key: 'missing_concept_pages',
    label: 'Missing pages',
    hint: 'Referenced by a [[link]] that resolves to no page.',
  },
  {
    key: 'stale_sources',
    label: 'Stale sources',
    hint: 'Ingested over 90 days ago with nothing linking to them.',
  },
];

type LintReportLists = Pick<
  LintResult['report'],
  'contradictions' | 'orphans' | 'missing_concept_pages' | 'stale_sources'
>;

export function LintDrawer({ open, onOpenChange }: Props) {
  const { primaryKbId, primaryKb } = useKnowledge();
  const { currentProvider, currentModel } = useModelAndProvider();
  const stream = useIngestStream();
  const { start, reset } = stream;

  const dispatchKbId = primaryKb?.id ?? null;
  const primaryKbUnavailable = Boolean(primaryKbId) && !dispatchKbId;

  const model = useMemo(
    () =>
      dispatchKbId
        ? resolveIngestModel(primaryKb?.default_model, currentProvider, currentModel)
        : null,
    [dispatchKbId, primaryKb?.default_model, currentProvider, currentModel]
  );

  const run = useCallback(() => {
    if (!dispatchKbId || !model) return;
    void start(`/knowledge/bases/${dispatchKbId}/lint`, { model });
  }, [dispatchKbId, model, start]);

  /**
   * Opening the drawer IS the request — the user picked "Check for problems",
   * not "show me a panel with a button in it".
   *
   * The ref is what keeps that from firing twice: `run` changes identity when
   * the model resolves, and an effect keyed on it would dispatch a second
   * agentic loop the moment the manifest arrived. It also clears the previous
   * base's report, for the same reason the ingest log clears — a finished
   * report is a claim about the base it ran against.
   */
  const startedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!open) return;
    if (!dispatchKbId || !model) return;
    if (startedFor.current === dispatchKbId) return;
    startedFor.current = dispatchKbId;
    reset();
    run();
  }, [open, dispatchKbId, model, run, reset]);

  useEffect(() => {
    if (startedFor.current && startedFor.current !== dispatchKbId) {
      startedFor.current = null;
      reset();
    }
  }, [dispatchKbId, reset]);

  const result = stream.finalResult as LintResult | null;
  const report = dispatchKbId && stream.status === 'done' ? (result?.report ?? null) : null;
  // Memoised, not `?? []` inline: a fresh literal every render would make the
  // grouping below re-run on every commit — and `report` is already a stable
  // reference for as long as the stream's terminal frame is.
  const items = useMemo<Diagnostic[]>(() => report?.diagnostics?.items ?? [], [report]);
  // `total` is the count BEFORE the daemon's cap, and it is the number that
  // goes on screen: a truncated list reporting its own length is how "3 errors"
  // gets rendered for a base with four hundred.
  const total = report?.diagnostics?.total ?? items.length;

  const grouped = useMemo(() => {
    const map = new Map<Severity, Diagnostic[]>(SEVERITIES.map((s) => [s, []]));
    for (const d of items) {
      const bucket = map.get(d.severity);
      if (bucket) bucket.push(d);
      else map.set(d.severity, [d]);
    }
    return SEVERITIES.map((severity) => ({ severity, rows: map.get(severity) ?? [] })).filter(
      (g) => g.rows.length > 0
    );
  }, [items]);

  const lists = useMemo(
    () =>
      report
        ? REPORT_LISTS.map((l) => ({ ...l, entries: report[l.key] ?? [] })).filter(
            (l) => l.entries.length > 0
          )
        : [],
    [report]
  );

  const running =
    Boolean(dispatchKbId && model) &&
    (stream.status === 'starting' || stream.status === 'streaming');
  const clean = report != null && total === 0 && lists.length === 0;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        // The header is a title plus the run control, and the line under it is a
        // run of live counts — neither is a description, so this takes Radix's
        // own explicit opt-out rather than invent copy for the accessibility
        // tree alone. Matches `ChangeLogDrawer`; see `ui/sheet.tsx` for why the
        // primitive cannot default it.
        aria-describedby={undefined}
        className="br-knowledge-drawer flex w-knowledge-rail-detail flex-col gap-0 p-0 sm:max-w-knowledge-rail-detail"
        data-testid="knowledge-lint-drawer"
      >
        {/* The run control sits in the HEADER, beside the title, not in the
            count row: the counts are a variable-width run of badges, and a
            button sharing that row is pushed off a 340px drawer by the third
            severity — which is exactly the base that most needs re-checking. */}
        <KnowledgePaneHeader
          sheet
          closeLabel={knowledgeCopy.drawers.closeLint}
          title={
            <SheetTitle className="min-w-0 truncate text-label">
              {knowledgeCopy.drawers.lint}
            </SheetTitle>
          }
          actions={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="shrink-0"
              data-testid="knowledge-lint-run"
              disabled={running || !dispatchKbId || !model}
              onClick={run}
            >
              {running ? 'Checking…' : report ? 'Check again' : 'Check'}
            </Button>
          }
        />

        {/* The counts. Pinned above the scroll box, because a count the reader
            has to scroll to find is the defect this drawer's sibling — the
            facet strip — was fixed for. Wrapping, because a base with all three
            severities plus the cap note does not fit one 340px line. */}
        <div className="flex flex-none flex-wrap items-center gap-2 border-b border-border-subtle px-4 py-2">
          {report ? (
            <>
              <span
                data-testid="knowledge-lint-count"
                className="text-supporting tabular-nums text-text-muted"
              >
                {total} {total === 1 ? 'finding' : 'findings'}
              </span>
              {grouped.map(({ severity, rows }) => (
                <Badge key={severity} tone={SEVERITY_TONE[severity]}>
                  {rows.length} {severity}
                </Badge>
              ))}
              {items.length < total && (
                <span className="text-supporting text-text-muted">first {items.length} shown</span>
              )}
            </>
          ) : (
            <span className="text-supporting text-text-muted">
              {running ? 'Reading every page in this base…' : 'Nothing checked yet.'}
            </span>
          )}
        </div>

        <div className="br-rows-flat min-h-0 flex-1 overflow-y-auto">
          {/* Two different reasons for one null, and only the second is a gap
              in the user's setup. Sending a coding-agent user to the Sources
              rail is the dead end described above: that picker lists nothing
              for them, so the instruction cannot be followed. Settings is the
              move that actually ends the situation, and it is named without a
              button because reaching `useNavigate` from this component would
              make it un-renderable outside a router — every test in
              `LintDrawer.test.tsx` renders it bare. */}
          {/* #109: this used to branch on whether the configured provider was a
              coding agent, because `resolveIngestModel` refused those and this
              drawer read the resulting `null` as "no model set up" — telling a
              user whose only provider was a coding agent to choose a model in a
              picker that same exclusion had emptied. Both halves of that loop
              are gone: the resolver no longer refuses by name, and a macro turn
              carries its tools over the MCP bridge. `null` here means what it
              says again. */}
          {primaryKbUnavailable ? (
            <EmptyState
              compact
              icon={AlertCircle}
              title="Knowledge base unavailable"
              description="Wait for this knowledge base to finish loading, or refresh your knowledge bases and try again."
            />
          ) : !model ? (
            <EmptyState
              compact
              icon={AlertCircle}
              title="No model is configured"
              description="A check reads the base with a model. Choose one in the Sources rail and try again."
            />
          ) : null}

          {model && running && (
            <div
              role="status"
              className="flex items-center gap-2 px-4 py-4 text-secondary text-text-muted"
            >
              <Spinner />
              <span>
                Checking this knowledge base
                {stream.events.length > 0 ? ` · ${stream.events.length} events` : ''}
              </span>
            </div>
          )}

          {model && stream.status === 'error' && (
            <EmptyState
              compact
              icon={AlertCircle}
              title="The check did not finish"
              description={stream.error ?? 'The stream ended without reporting a result.'}
              actions={
                <Button variant="secondary" onClick={run}>
                  Try again
                </Button>
              }
            />
          )}

          {clean && (
            <EmptyState
              compact
              icon={CheckCircle2}
              title="Nothing to fix"
              description="Every page is inside the vocabulary, linked, and current."
            />
          )}

          {report &&
            grouped.map(({ severity, rows }) => {
              const Icon = SEVERITY_ICON[severity];
              return (
                <section key={severity} aria-label={SEVERITY_LABEL[severity]}>
                  <h3 className="flex items-center gap-2 bg-background-fill px-4 py-1.5 text-caps text-text-muted">
                    <Icon aria-hidden="true" className="h-icon-row w-icon-row" />
                    {SEVERITY_LABEL[severity]}
                    <span className="tabular-nums">{rows.length}</span>
                  </h3>
                  {/* The rows sit 8px in from the drawer's edges, so the hover wash is a rounded row
                      and not a full-bleed slab; their text stays on the header's 16px edge. */}
                  <div className="px-2">
                    {rows.map((d, i) => (
                      <div
                        key={`${d.rule}-${d.subject}-${i}`}
                        data-testid="knowledge-lint-diagnostic"
                        className="biorouter-list-row flex flex-col gap-1 px-2 py-3"
                      >
                        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
                          {/* The RULE ID, not the message. It is stable across
                              releases and the message is prose that will be
                              reworded — so it is the half a user can search for,
                              quote in an issue, or match a fix against. */}
                          <span className="shrink-0 font-mono text-supporting text-text-muted">
                            {d.rule}
                          </span>
                          {/* WRAPPED, not truncated. The subject is a page's
                              identifier or a whole edge rendered as
                              `<subject> -<predicate>-> <object>`; clipped to one
                              line, the half that names what the finding is about
                              is the half that disappears. */}
                          <span className="min-w-0 break-words text-label text-text-default">
                            {d.subject}
                          </span>
                        </div>
                        <p className="min-w-0 break-words text-body text-text-muted">{d.message}</p>
                        {d.path && (
                          <p className="min-w-0 break-all font-mono text-supporting text-text-muted">
                            {d.path}
                          </p>
                        )}
                      </div>
                    ))}
                  </div>
                </section>
              );
            })}

          {lists.map((l) => (
            <section key={l.key} aria-label={l.label}>
              {/* The count joins the caps run, and the one-line hint is the
                  heading's InfoTip (spec 3.12): a sibling of the `<h3>`, so the
                  heading's name stays the list's name. */}
              <div className="flex items-center gap-1 bg-background-fill px-4 py-1.5">
                <h3 className="flex items-center gap-2 text-caps text-text-muted">
                  {l.label}
                  <span className="tabular-nums">{l.entries.length}</span>
                </h3>
                <InfoTip label={l.label} help={l.hint} side="left" />
              </div>
              <ul className="flex flex-col gap-1 px-4 py-2">
                {l.entries.map((entry) => (
                  <li key={entry} className="min-w-0 break-all font-mono text-supporting">
                    {entry}
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </SheetContent>
    </Sheet>
  );
}
