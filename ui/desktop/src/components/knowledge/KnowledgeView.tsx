// ui/desktop/src/components/knowledge/KnowledgeView.tsx
import { useEffect, useState } from 'react';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ReadableContent } from '../Layout/ReadableContent';
import {
  ClipboardList,
  Download,
  FolderOpen,
  History,
  KnowledgeIcon,
  MoreHorizontal,
  RefreshCw,
  Target,
  Trash2,
} from '../icons/app-icons';
import { getLocation } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { Button } from '../ui/button';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { EmptyState } from '../ui/empty-state';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { KBSelectorTrigger } from './KBSelector/KBSelectorTrigger';
import { KBManagerDialog } from './KBSelector/KBManagerDialog';
import { SourcesRail } from './SourcesRail';
import { KnowledgeGraphPanel } from './graph/KnowledgeGraphPanel';
import { ChangeLogDrawer } from './changelog/ChangeLogDrawer';
import { LintDrawer } from './lint/LintDrawer';
import { useKnowledge } from './KnowledgeContext';
import { useKnowledgeGraph } from './hooks/useKnowledgeGraph';
import { useKnowledgeBases } from './hooks/useKnowledgeBases';
import { kbFormatLabel, LEGACY_FORMAT_TITLE } from './kbFormat';
import { Badge } from '../ui/badge';

export default function KnowledgeView() {
  return <KnowledgeViewInner />;
}

/**
 * The Knowledge section shell (ui-spec §3).
 *
 * Three bands over one measure: a header band, a subject band naming the base
 * the whole view is about, and the workspace. All three take
 * `ReadableContent size="graph"` — `--measure-graph` — so their left edges are
 * one line; the band hairlines are FULL-BLEED, outside the measure.
 *
 * ⚠ **`size="graph"` is a deliberate widening, not a repair.** Knowledge is the
 * app's one canvas view and gets the app's one canvas measure. The gutters are
 * what this flattens: the header used to run
 * `px-4 pb-4 pt-8 sm:px-6 lg:px-8 lg:pb-6 lg:pt-12` against the app's flat
 * `px-8`.
 *
 * ⚠ **The subject band is NOT `--chrome-height`.** The three 44px chrome bands
 * (sidebar titlebar, chat header, artifact strip) meet at one continuous top
 * edge and move together or not at all. This band is inside the page, below the
 * page header, and takes the content row rhythm (`h-row`) instead.
 *
 * ⚠ **`page-transition` is not carried forward.** It has ten call sites in this
 * app and ZERO matching rules in `src/styles/`, and the design system publishes
 * "Dead Tailwind classes: 0" as a standing check — so an eleventh call site
 * would falsify a published number to no effect.
 */
function KnowledgeViewInner() {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
  const [managerStartsInCreate, setManagerStartsInCreate] = useState(false);
  const [changeLogOpen, setChangeLogOpen] = useState(false);
  const [lintOpen, setLintOpen] = useState(false);
  const [previewSha, setPreviewSha] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [compactView, setCompactView] = useState<'digest' | 'graph'>('graph');

  const { refresh, bases, primaryKb, primaryKbId, registerGraphRefresh } = useKnowledge();
  const { graph, loading, error, refresh: refreshGraph } = useKnowledgeGraph(primaryKbId);
  const { exportArchive, remove } = useKnowledgeBases();

  // The KnowledgeProvider only fetches the base list once at app start, so a
  // knowledge base created elsewhere (e.g. via chat / the knowledge MCP tools)
  // would not appear here until a full reload. Re-fetch whenever the Knowledge
  // view mounts so navigating to this tab always reflects the current bases.
  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Expose the graph refresh to KnowledgeContext so the ingest panel can call it
  // after each digest.
  useEffect(() => {
    registerGraphRefresh(refreshGraph);
    return () => registerGraphRefresh(null);
  }, [refreshGraph, registerGraphRefresh]);

  // ⌘K is "switch base", and always has been — so it opens the PICKER, not the
  // manager.
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setPickerOpen((v) => !v);
      }
    }
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, []);

  function openManager(inCreate: boolean) {
    setManagerStartsInCreate(inCreate);
    setManagerOpen(true);
  }

  async function openKbFolder() {
    if (!primaryKbId) return;
    try {
      const res = await getLocation({
        path: { id: primaryKbId },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const path = res.data?.path;
      if (path) await window.electron.openDirectoryInExplorer(path);
    } catch (err) {
      window.electron.logInfo(`Failed to open knowledge base folder: ${String(err)}`);
    }
  }

  async function confirmDelete() {
    if (!primaryKb || deleting) return;
    setDeleting(true);
    try {
      await remove(primaryKb.id);
      setConfirmingDelete(false);
    } finally {
      setDeleting(false);
    }
  }

  const noBasesAtAll = bases.length === 0;

  return (
    <MainPanelLayout>
      {/* ⚠ **THE CONTAINER IS HERE, NOT ON THE GRID** (R-08), and the two are
          not interchangeable: `@container` styles apply to a container's
          DESCENDANTS, never to the container element itself. An earlier draft
          put `br-knowledge-pane` and `br-knowledge-body` on the same div, so
          the grid could never restyle itself and every step silently produced
          one column. It also has to sit ABOVE the header and subject bands —
          the height step yields the header's description line and the two-column
          step hides the Sources/Graph tabs, and neither is inside the
          workspace. Measured in the browser; jsdom evaluates no container
          queries at all. */}
      <div
        className="br-knowledge-pane relative flex min-w-0 flex-1 flex-col overflow-hidden"
        data-search-scroll-area
      >
        {/* HEADER BAND — TITLE ONLY (R-01). The hairline is FULL-BLEED.

            ⚠ **The selector and `Manage bases` used to live on this row**, and
            deleting them is the fix rather than a simplification. They were not
            "above the banner" as reported — they were the second child of this
            title row, co-linear with the `h1`. The real defect was that the base
            was identified TWICE, 40px apart: once here and again in the subject
            band below, which already draws the dot, the name, the format badge,
            the privacy badge and the counts. One subject, stated once.

            Base selection now lives in the subject band, whose name IS the
            switcher; `Manage bases` moved into that picker's own footer. */}
        <div className="flex-shrink-0 border-b border-border-subtle">
          <ReadableContent size="graph" className="px-8 pb-6 pt-12">
            <h1 className="mb-1 text-title">Knowledge</h1>
            {/* Yields below 620px of PANE height (R-08). At the app's 600px
                minimum window the pane is 568px, so this is the default state
                there rather than an edge case. */}
            <p className="br-knowledge-subtitle text-secondary text-text-muted">
              Personal knowledge bases Biorouter builds and maintains for you.
            </p>
          </ReadableContent>
        </div>

        {/*
          ONE `Tabs` root spanning the subject band and the workspace, because
          the triggers live in the band and the panels live in the workspace and
          Radix links them by id. Two roots would render two working tab strips
          whose `aria-controls` pointed at nothing.

          `text-text-default` is not decoration: the primitive's root sets
          `text-text-muted`, which every unstyled descendant would otherwise
          inherit — and the whole workspace is inside it.
        */}
        <Tabs
          value={compactView}
          onValueChange={(v) => setCompactView(v as 'digest' | 'graph')}
          className="flex min-h-0 flex-1 flex-col text-text-default"
        >
          {/* SUBJECT BAND — one `h-row` row naming the base the view is about. */}
          <div className="flex-shrink-0 border-b border-border-subtle">
            <ReadableContent size="graph" className="px-8">
              <div className="flex h-row items-center justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  {primaryKb ? (
                    <>
                      {/* R-01: the base's NAME is the switcher. One control
                          carrying the dot, the name and a caret — the trigger
                          renders them itself. */}
                      <KBSelectorTrigger
                        variant="subject"
                        open={pickerOpen}
                        onOpenChange={setPickerOpen}
                        onManage={() => openManager(false)}
                        onCreate={() => openManager(true)}
                      />
                      {/* `schema_version` is folded in, not just `format`: the
                          field is `serde(default)` on the daemon so a legacy
                          manifest reads `okf`, and a badge written against it
                          alone calls every pre-OKF base OKF. */}
                      <Badge
                        uppercase
                        className="shrink-0"
                        title={
                          kbFormatLabel(primaryKb) === 'Legacy' ? LEGACY_FORMAT_TITLE : undefined
                        }
                      >
                        {kbFormatLabel(primaryKb)}
                      </Badge>
                      {primaryKb.tier !== 'public' && <PrivacyBadge tier={primaryKb.tier} dense />}
                      {graph && (
                        <span
                          data-testid="knowledge-graph-summary"
                          className="shrink-0 text-supporting font-mono tabular-nums text-text-muted"
                        >
                          {graph.nodes.length} {graph.nodes.length === 1 ? 'page' : 'pages'} ·{' '}
                          {graph.edges.length} {graph.edges.length === 1 ? 'link' : 'links'}
                        </span>
                      )}
                    </>
                  ) : (
                    <span className="truncate text-label text-text-muted">
                      No primary knowledge base
                    </span>
                  )}

                  {/* ⚠ **AFTER the base, not before it.** The tabs used to open this row, so
                    at the narrow step — the only step where they exist — the page's
                    SUBJECT was the second thing read, behind a control that merely
                    switches which half of it you are looking at. The band's job is to
                    name the base; the pair lives here so the band still can.

                    Below `--breakpoint-md` the workspace is a two-tab pair, and
                    the pair lives HERE so the band still names the base. The
                    `<Tabs>` primitive supplies role="tablist", the roving focus
                    and ←/→/Home/End; the hand-rolled segmented pill this
                    replaces is design system D-07 option B, whose own record
                    reads "Delete B." */}
                  {/* ⚠ **`ms-2 border-s ps-3` is a GROUP BREAK, not decoration.**
                    Measured at the 760px pane: the band separates every child by
                    the same 8px, so the tabs began 8px after `38 links` and the
                    row read as one run — "43 pages · 38 links Sources Graph" —
                    which files a MODE SWITCHER among the base's attributes. The
                    identity group describes the subject; the pair chooses which
                    half of it you see. A rule plus a wider gap is the smallest
                    thing that says those are two kinds of thing. */}
                  <TabsList className="br-knowledge-tabs ms-2 gap-4 border-s border-b-0 border-border-subtle ps-3">
                    <TabsTrigger value="digest">Sources</TabsTrigger>
                    <TabsTrigger value="graph">Graph</TabsTrigger>
                  </TabsList>
                </div>

                {/* Two visible actions against the app's ceiling of three, with
                  every destructive action behind the one `⋯` (ROWS-3). */}
                <div className="flex shrink-0 items-center gap-2">
                  {/* The third of the three. A check is READ-ONLY, so it does
                      not belong behind the `⋯` with the destructive actions —
                      and it was previously behind nothing at all: the route and
                      its generated client both shipped with no caller, so the
                      format chooser promised a validation the user could not
                      run. */}
                  <Button
                    variant="ghost"
                    shape="round"
                    onClick={() => setLintOpen(true)}
                    disabled={!primaryKbId}
                    aria-label="Check for problems"
                    title="Check for problems"
                  >
                    <ClipboardList aria-hidden="true" />
                  </Button>
                  <Button
                    variant="ghost"
                    shape="round"
                    onClick={() => void refreshGraph()}
                    disabled={!primaryKbId || loading}
                    aria-label="Refresh graph"
                    title="Refresh graph"
                  >
                    <RefreshCw className={loading ? 'animate-spin' : ''} aria-hidden="true" />
                  </Button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        shape="round"
                        disabled={!primaryKbId}
                        aria-label="More knowledge base actions"
                      >
                        <MoreHorizontal aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onSelect={() =>
                          primaryKb && void exportArchive(primaryKb.id, primaryKb.name)
                        }
                      >
                        <Download aria-hidden="true" />
                        Export as .brkb
                      </DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => setChangeLogOpen(true)}>
                        <History aria-hidden="true" />
                        Change log
                      </DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => void openKbFolder()}>
                        <FolderOpen aria-hidden="true" />
                        Open folder
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setConfirmingDelete(true)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete knowledge base
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </div>
            </ReadableContent>
          </div>

          {/* WORKSPACE */}
          <div className="flex min-h-0 flex-1 flex-col">
            <ReadableContent size="graph" className="flex min-h-0 flex-1 flex-col px-8 pb-8 pt-6">
              {noBasesAtAll ? (
                <EmptyState
                  icon={KnowledgeIcon}
                  title="No knowledge bases yet"
                  description="Create one and Biorouter will build and maintain notes, sources and links for you."
                  actions={
                    <>
                      <Button onClick={() => openManager(true)}>Create knowledge base</Button>
                      <Button variant="secondary" onClick={() => openManager(false)}>
                        Import .brkb
                      </Button>
                    </>
                  }
                />
              ) : !primaryKbId ? (
                <EmptyState
                  icon={Target}
                  title="No primary knowledge base"
                  description="Choose which base this chat reads and writes."
                  actions={<Button onClick={() => openManager(false)}>Choose a base</Button>}
                />
              ) : (
                /* ⚠ **CONTAINER queries, not media queries** (R-08). The section
                   shipped with one `md:` breakpoint at 930px, and it tests the
                   VIEWPORT while the thing that changes size is this PANE.
                   `main.ts` derives `minWidth: 1000` as 240px of sidebar plus a
                   760px column, so at the app's own minimum window with the
                   sidebar open the viewport is 1000 — `md:` fires — while the
                   pane is 760, and the 300px Sources rail leaves 444px for a
                   filter strip whose content measures 757px. The section was at
                   its most broken at the smallest size the app allows.

                   The ladder lives in `main.css` under `.br-knowledge-pane`;
                   the classes here only mark the parts. */
                <div className="br-knowledge-body min-h-0 flex-1 gap-4">
                  <TabsContent
                    forceMount
                    value="digest"
                    data-testid="knowledge-digest-panel"
                    className={`br-knowledge-sources ${compactView === 'digest' ? 'flex' : 'hidden'} mt-0 min-h-0 flex-col`}
                  >
                    <SourcesRail
                      className="flex-1"
                      kb={
                        primaryKb
                          ? { id: primaryKb.id, name: primaryKb.name, tier: primaryKb.tier }
                          : null
                      }
                    />
                  </TabsContent>

                  <TabsContent
                    forceMount
                    value="graph"
                    data-testid="knowledge-graph-panel"
                    className={`br-knowledge-graph ${compactView === 'graph' ? 'flex' : 'hidden'} mt-0 min-h-0 min-w-0`}
                  >
                    <KnowledgeGraphPanel
                      kbId={primaryKbId}
                      graph={graph}
                      loading={loading}
                      error={error}
                      onRefresh={() => void refreshGraph()}
                      previewSha={previewSha}
                      onClearPreview={() => setPreviewSha(null)}
                    />
                  </TabsContent>
                </div>
              )}
            </ReadableContent>
          </div>
        </Tabs>

        <KBManagerDialog
          open={managerOpen}
          onOpenChange={setManagerOpen}
          startInCreate={managerStartsInCreate}
        />

        <LintDrawer open={lintOpen} onOpenChange={setLintOpen} />

        <ChangeLogDrawer
          open={changeLogOpen}
          onOpenChange={setChangeLogOpen}
          onPreview={(sha) => {
            setPreviewSha(sha);
            setChangeLogOpen(false);
          }}
          onRestored={() => setChangeLogOpen(false)}
        />

        <ConfirmationModal
          isOpen={confirmingDelete}
          title={`Delete "${primaryKb?.name ?? ''}"?`}
          message="This permanently removes the knowledge base and cannot be undone."
          confirmLabel="Delete"
          cancelLabel="Cancel"
          confirmVariant="destructive"
          isSubmitting={deleting}
          onConfirm={() => void confirmDelete()}
          onCancel={() => setConfirmingDelete(false)}
        />
      </div>
    </MainPanelLayout>
  );
}
