// ui/desktop/src/components/knowledge/KnowledgeView.tsx
import { useEffect, useState, type AnimationEvent } from 'react';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import {
  ClipboardList,
  Download,
  FolderOpen,
  History,
  MoreHorizontal,
  RefreshCw,
  Trash2,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { getLocation } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { Button } from '../ui/button';
import { EmptyState } from '../ui/empty-state';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { InfoTip } from '../ui/info-tip';
import { RowActionMenuItems, type RowActionItem } from '../ui/row-actions';
import { Spinner } from '../ui/spinner';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { Badge } from '../ui/badge';
import { KBSelectorTrigger } from './KBSelector/KBSelectorTrigger';
import { KBManagerDialog } from './KBSelector/KBManagerDialog';
import { useSelectorFocusReturn } from './KBSelector/useSelectorFocusReturn';
import { KbTierPanel } from './KbTierControl';
import { SourcesRail } from './SourcesRail';
import { KnowledgeGraphPanel } from './graph/KnowledgeGraphPanel';
import { ChangeLogDrawer } from './changelog/ChangeLogDrawer';
import { LintDrawer } from './lint/LintDrawer';
import { useKnowledge } from './KnowledgeContext';
import { useKnowledgeGraph } from './hooks/useKnowledgeGraph';
import { useKnowledgeBases } from './hooks/useKnowledgeBases';
import { kbFormatLabel, LEGACY_FORMAT_TITLE } from './kbFormat';
import { knowledgeCopy } from './copy';

type CompactView = 'digest' | 'graph';

export default function KnowledgeView() {
  return <KnowledgeViewInner />;
}

/** The Refresh action's glyph while the graph reloads: the one busy glyph, not a spun icon. */
function RefreshingGlyph() {
  return <Spinner />;
}

/**
 * The Knowledge section shell (spec 3.12; owner message 6: keep the structure and the logic,
 * simplify the skin).
 *
 * **One 44px band** (`PageHeader`, the app's one band implementation) over a workspace of two
 * flush columns. The band merges what used to be two: a 24px title header with a subtitle, and a
 * 40px subject band naming the base. It now reads, left to right, `Knowledge` (the `<h1>`, with
 * its description in an InfoTip), `/`, the base's name (which IS the switcher), the format badge,
 * the tier badge-button, the counts, the Sources/Graph tabs at the narrow step, and the actions.
 *
 * ⚠ **The tier is in the band so it is visible at every width.** It used to sit at the top of the
 * Sources rail, which hides behind the default Graph tab below the 860px step: on a narrow pane
 * the base's tier and its control were both off screen.
 *
 * ⚠ **`page-transition` is not carried forward.** It has call sites in this app and ZERO
 * matching rules in `src/styles/`, so another call site would be a dead class.
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
  const [compactView, setCompactView] = useState<CompactView>('graph');
  // The tab a PERSON just picked, until its enter animation has played. A tab panel fades in only
  // for that (principle 9): the layout can also flip a panel from `display: none` to shown (the
  // 860px step forces both on, and the narrow step hides one again), and the browser restarts a
  // panel's animation every time its `display` leaves `none`, so without this a window drag across
  // the step replayed the Sources fade.
  const [enteringTab, setEnteringTab] = useState<CompactView | null>(null);

  const { refresh, bases, primaryKb, primaryKbId, registerGraphRefresh } = useKnowledge();
  const { graph, loading, error, refresh: refreshGraph } = useKnowledgeGraph(primaryKbId);
  const { exportArchive, remove } = useKnowledgeBases();
  // The picker's row that opens the manager is gone when the manager closes, so the view hands
  // keyboard focus back to the selector itself (V3-R7-3).
  const selectorFocus = useSelectorFocusReturn();

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
    selectorFocus.opened();
    setManagerStartsInCreate(inCreate);
    setManagerOpen(true);
  }

  function changeManagerOpen(next: boolean) {
    if (!next) selectorFocus.closed();
    setManagerOpen(next);
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

  // `animationend` bubbles from anything animating inside the panel (a spinner, a skeleton), and
  // only the panel's own enter ends the flag.
  function endEnter(event: AnimationEvent<HTMLDivElement>) {
    if (event.target === event.currentTarget) setEnteringTab(null);
  }

  // Every destructive action stays behind the one `⋯` (ROWS-3); the read-only check and the
  // refresh are the two visible actions.
  const menuItems: RowActionItem[] = [
    {
      label: knowledgeCopy.actions.export,
      icon: Download,
      onSelect: () => primaryKb && void exportArchive(primaryKb.id, primaryKb.name),
    },
    {
      label: knowledgeCopy.actions.changeLog,
      icon: History,
      onSelect: () => setChangeLogOpen(true),
    },
    {
      label: knowledgeCopy.actions.openFolder,
      icon: FolderOpen,
      onSelect: () => void openKbFolder(),
    },
    { kind: 'separator' },
    {
      label: knowledgeCopy.actions.delete,
      icon: Trash2,
      destructive: true,
      onSelect: () => setConfirmingDelete(true),
    },
  ];

  const formatLabel = primaryKb ? kbFormatLabel(primaryKb) : null;

  return (
    <MainPanelLayout removeTopPadding>
      {/* ⚠ **THE CONTAINER IS HERE, NOT ON THE GRID** (R-08), and the two are
          not interchangeable: `@container` styles apply to a container's
          DESCENDANTS, never to the container element itself. It also has to sit
          ABOVE the band, because the two-column step hides the band's
          Sources/Graph tabs. Measured in the browser; jsdom evaluates no
          container queries at all. */}
      <div
        className="br-knowledge-pane relative flex min-w-0 flex-1 flex-col overflow-hidden"
        data-search-scroll-area
      >
        {/*
          ONE `Tabs` root spanning the band and the workspace, because the
          triggers live in the band and the panels live in the workspace and
          Radix links them by id. Two roots would render two working tab strips
          whose `aria-controls` pointed at nothing.

          `text-text-default` is not decoration: the primitive's root sets
          `text-text-muted`, which every unstyled descendant would otherwise
          inherit — and the whole workspace is inside it.
        */}
        <Tabs
          value={compactView}
          onValueChange={(v) => {
            setCompactView(v as CompactView);
            setEnteringTab(v as CompactView);
          }}
          className="flex min-h-0 flex-1 flex-col text-text-default"
        >
          <PageHeader
            title={knowledgeCopy.title}
            info={knowledgeCopy.viewInfo}
            adornment={
              <>
                <span aria-hidden="true" className="text-text-subtle">
                  /
                </span>
                {primaryKb ? (
                  <>
                    {/* The base's NAME is the switcher: one control carrying the
                        dot, the name and a caret (Crew's channel title). */}
                    <KBSelectorTrigger
                      variant="subject"
                      open={pickerOpen}
                      onOpenChange={setPickerOpen}
                      onManage={() => openManager(false)}
                      onCreate={() => openManager(true)}
                      triggerRef={selectorFocus.triggerRef}
                    />
                    {/* `schema_version` is folded in, not just `format`: the
                        field is `serde(default)` on the daemon so a legacy
                        manifest reads `okf`, and a badge written against it
                        alone calls every pre-OKF base OKF. Legacy explains
                        itself in an InfoTip (it used to be a native `title`
                        no keyboard could reach). */}
                    {formatLabel === 'Legacy' ? (
                      <InfoTip
                        label={knowledgeCopy.legacyFormatName}
                        help={LEGACY_FORMAT_TITLE}
                        side="bottom"
                        asChild
                      >
                        <Badge data-testid="knowledge-format-badge">{formatLabel}</Badge>
                      </InfoTip>
                    ) : (
                      <Badge data-testid="knowledge-format-badge">{formatLabel}</Badge>
                    )}
                    <KbTierPanel
                      kb={{ id: primaryKb.id, name: primaryKb.name, tier: primaryKb.tier }}
                    />
                    {graph && (
                      <span data-testid="knowledge-graph-summary" className="shrink-0">
                        {knowledgeCopy.summary(graph.nodes.length, graph.edges.length)}
                      </span>
                    )}
                  </>
                ) : (
                  <span className="truncate text-label text-text-muted">
                    {knowledgeCopy.noPrimary}
                  </span>
                )}
              </>
            }
            tabs={
              // Below the 860px step the workspace is a two-tab pair. `border-b-0`:
              // the band's hairline is the one rule, and the underline lands on it.
              <TabsList className="br-knowledge-tabs border-b-0">
                <TabsTrigger value="digest">{knowledgeCopy.tabs.sources}</TabsTrigger>
                <TabsTrigger value="graph">{knowledgeCopy.tabs.graph}</TabsTrigger>
              </TabsList>
            }
            actions={
              <>
                {/* A check is READ-ONLY, so it does not belong behind the `⋯`
                    with the destructive actions. */}
                <PageHeaderAction
                  icon={ClipboardList}
                  label={knowledgeCopy.actions.check}
                  onClick={() => setLintOpen(true)}
                  disabled={!primaryKbId}
                />
                <PageHeaderAction
                  icon={loading ? RefreshingGlyph : RefreshCw}
                  label={knowledgeCopy.actions.refresh}
                  onClick={() => void refreshGraph()}
                  // `busy`, not `disabled`, while the request this button started is out: a
                  // button that turns `disabled` under a keyboard user hands focus to <body>
                  // (V3-R2-1). `busy` keeps it focused and ignores the press itself. No base
                  // to refresh is a different thing: nothing was pressed, and it stays disabled.
                  disabled={!primaryKbId}
                  busy={loading}
                />
                <DropdownMenu>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          shape="round"
                          disabled={!primaryKbId}
                          aria-label={knowledgeCopy.actions.more}
                        >
                          <MoreHorizontal aria-hidden="true" />
                        </Button>
                      </DropdownMenuTrigger>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">{knowledgeCopy.actions.more}</TooltipContent>
                  </Tooltip>
                  <DropdownMenuContent align="end">
                    <RowActionMenuItems items={menuItems} />
                  </DropdownMenuContent>
                </DropdownMenu>
              </>
            }
          />

          {/* WORKSPACE: edge to edge under the band, two flush columns. */}
          <div className="flex min-h-0 flex-1 flex-col">
            {noBasesAtAll ? (
              <div className="flex min-h-0 flex-1 overflow-auto px-6">
                <EmptyState
                  className="m-auto"
                  icon={ENTITY_ICONS.knowledge}
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
              </div>
            ) : !primaryKbId ? (
              <div className="flex min-h-0 flex-1 overflow-auto px-6">
                <EmptyState
                  className="m-auto"
                  icon={ENTITY_ICONS.knowledge}
                  title="No primary knowledge base"
                  description="Choose which base this chat reads and writes."
                  actions={<Button onClick={() => openManager(false)}>Choose a base</Button>}
                />
              </div>
            ) : (
              /* ⚠ **CONTAINER queries, not media queries** (R-08). The thing
                 that changes size is this PANE, not the viewport: at the app's
                 own minimum window with the sidebar open the viewport is wide
                 enough for a `md:` query while the pane is 760px. The ladder
                 lives in `main.css` under `.br-knowledge-pane`; the classes
                 here only mark the parts. */
              <div className="br-knowledge-body min-h-0 flex-1">
                <TabsContent
                  forceMount
                  value="digest"
                  data-testid="knowledge-digest-panel"
                  data-entering={enteringTab === 'digest' ? '' : undefined}
                  onAnimationEnd={endEnter}
                  className={`br-knowledge-sources ${compactView === 'digest' ? 'flex' : 'hidden'} mt-0 min-h-0 flex-col`}
                >
                  <SourcesRail className="flex-1" />
                </TabsContent>

                <TabsContent
                  forceMount
                  value="graph"
                  data-testid="knowledge-graph-panel"
                  data-entering={enteringTab === 'graph' ? '' : undefined}
                  onAnimationEnd={endEnter}
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
          </div>
        </Tabs>

        <KBManagerDialog
          open={managerOpen}
          onOpenChange={changeManagerOpen}
          startInCreate={managerStartsInCreate}
          onCloseAutoFocus={selectorFocus.onCloseAutoFocus}
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
