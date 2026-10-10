import { hasSelectedText } from '../../utils/previewTextSelection';
import { QuotedTextSelection, useTextSelection, attachSelectedText } from '../QuotedTextSelection';
import { usePreviewMotion } from './usePreviewMotion';
import { DUR, prefersReducedMotion } from '../../styles/motion';
import { UIResourceRenderer } from '@mcp-ui/client';
import {
  type CSSProperties,
  type PointerEvent,
  type SyntheticEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { useTheme, useThemeFamily } from '../../contexts/ThemeContext';
import {
  CODE_FONT_FAMILY,
  codeThemesByFamily,
  GUTTER_INK_MIX,
  withFadedGutter,
} from '../../styles/codeTheme';
import { cn } from '../../utils';
import { injectArtifactBrowserCsp } from '../../utils/artifactSecurity';
import { withPreviewActivityTracking } from '../../utils/previewActivity';
import { PREVIEW_SIZE_MESSAGE_TYPE, withPreviewSizeReporting } from '../../utils/previewSize';
import { sendArtifactAnnotation } from '../../utils/annotationChannel';
import { artifactFileErrorMessage } from '../../utils/artifactFileErrors';
import { copyToClipboard } from '../../utils/clipboard';
import { useTransientValue } from '../../hooks/useTransientFlag';
import { describeUnsupportedFormat } from '../../utils/formatSupport';
import { isImageExtension } from '../../utils/imageFormats';
import {
  isTabCycleEvent,
  tabCycleOffset,
  nextTabIndex,
  isWithinArtifactPanel,
  ARTIFACT_PANEL_ATTR,
} from '../../utils/tabCycle';
import {
  AlertTriangle,
  Camera,
  ChartColumn,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Code,
  Copy,
  ExternalLink,
  File,
  FileText,
  FileX,
  Folder,
  Github,
  Globe,
  Image,
  Lock,
  MessageSquareText,
  MoreHorizontal,
  RefreshCw,
  Search,
  X,
} from '../icons/app-icons';
import { useTabStripOverflow } from '../Layout/useTabStripOverflow';
import type { PreviewPanelMode } from '../Layout/yieldLadder';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { Button, buttonVariants } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import { SegmentedControl } from '../ui/segmented-control';
import { PREVIEW_COPY } from './copy';
import {
  PreviewBar,
  PreviewBarActions,
  PreviewBarMeta,
  PreviewBarScope,
  PreviewIconButton,
  PreviewSkeleton,
} from './PreviewBar';
import './preview-panel.css';
import AnnotationOverlay, { type SelectedRegion } from './AnnotationOverlay';
import DelimitedTable from './DelimitedTable';
import { annotateBrowserReason } from './captureOnBrowser';
import DocumentPreview from './DocumentPreview';
import MarkdownDocument from './MarkdownDocument';
import WebPagePreview, { type LiveBrowserShare } from './WebPagePreview';
import NotebookPreview from './NotebookPreview';
import type {
  ArtifactFileEntry,
  ArtifactFilePreview,
  ArtifactGitEntry,
  ArtifactGitStatus,
  ArtifactSource,
} from './artifactTypes';
import {
  basenameFromPath,
  dirnameFromPath,
  extensionFromPath,
  imageSourceForPreview,
  isDelimitedPath,
  isMarkdownPath,
  languageForText,
  PAPER_GUTTER_EM,
  parseDelimitedTable,
  withHostTheme,
} from './artifactUtils';

// Enough rows to see the shape of the data; a 200k-row CSV must not lock up the
// renderer just because the agent wrote it.
const MAX_TABLE_ROWS = 500;

// Line numbers stop helping once a file is long enough that nobody is counting.
const MAX_LINE_NUMBERED_LINES = 5_000;

const MAX_TIFF_DIMENSION = 8_192;
const MAX_TIFF_PIXELS = 32_000_000;
const MAX_TIFF_RGBA_BYTES = 128_000_000;

export function safeTiffDimensions(page: { width: number; height: number }) {
  const { width, height } = page;
  if (
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0 ||
    width > MAX_TIFF_DIMENSION ||
    height > MAX_TIFF_DIMENSION
  ) {
    throw new Error('TIFF dimensions exceed the safe preview limit.');
  }
  const pixels = width * height;
  const rgbaBytes = pixels * 4;
  if (pixels > MAX_TIFF_PIXELS || rgbaBytes > MAX_TIFF_RGBA_BYTES) {
    throw new Error('TIFF dimensions exceed the safe preview limit.');
  }
  return { width, height, rgbaBytes };
}

// Text files conventionally end with a newline. Left in, it renders a phantom
// last line — numbered, empty, and one more than the file actually has.
function stripTrailingNewline(text: string): string {
  return text.replace(/\r?\n$/, '');
}

/**
 * An iframe is held transparent until its document has loaded (P0 3), then the
 * stylesheet fades it in. Written to the DOM rather than kept in state: the
 * frame is the thing that knows, and a re-render must not reset it.
 */
function markFrameLoaded(event: SyntheticEvent<HTMLIFrameElement>) {
  event.currentTarget.dataset.loaded = 'true';
}

function countLines(text: string): number {
  const body = stripTrailingNewline(text);
  return body === '' ? 0 : body.split('\n').length;
}

/** The one mono stack (design.md §3.2), shared with chat code blocks and the terminal. */
const CODE_FONT = CODE_FONT_FAMILY;

// The header's actions are `PreviewIconButton`s (PreviewBar.tsx): the shared
// ghost square `Button` at 32px, the same control every 44px band uses, so the
// panel's actions can no longer drift from the chat header's by hand.

interface ArtifactViewerProps {
  artifact: ArtifactSource | null;
  isOpen?: boolean;
  motionReady?: boolean;
  isResizing?: boolean;
  onClose: () => void;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  onResizeStart?: (event: PointerEvent<HTMLDivElement>) => void;
  onRenderError?: (error: ArtifactRenderError) => void;
  /** Present only while the user has explicitly shared a live web tab. */
  onLiveBrowserShareChange?: (share: LiveBrowserShare | null) => void;
  /** Exact revision of the file bytes currently rendered in this panel. */
  onFilePreviewRevisionChange?: (revision: string | null) => void;
  /**
   * The chat this panel belongs to. Chat-only, exactly like `onRenderError`:
   * a read-only transcript passes nothing, so the annotate control never
   * appears there — annotating a saved session would attach a region to a
   * conversation that is not running.
   */
  sessionId?: string;
  refreshRevision?: number;
  className?: string;
  style?: CSSProperties;
  /**
   * Rung 2 of the yield ladder: a column beside the conversation, or a sheet
   * above it. The host decides (`useArtifactPanel`) and places the panel with
   * CSS; the panel only needs it to name its resize edge's orientation. Nothing
   * here renders differently by layout — a crossing must not remount anything.
   */
  layout?: PreviewPanelMode;
  /** Stack only: the sheet is folded to its strip. */
  folded?: boolean;
  onToggleFold?: () => void;
  /** A tab click or a click on the bare strip unfolds a folded sheet. */
  onUnfold?: () => void;
  /**
   * The height this panel needs to show its content whole (strip included), or
   * null when the content cannot say (an image, a live page, a directory). Called
   * only once the content is READY — never for the loading placeholder — so the
   * host can hold a fresh sheet back until it knows how tall to make it.
   */
  onContentHeightChange?: (height: number | null) => void;
}

/** The two frame names a preview's document can live in. */
const PREVIEW_FRAME_SELECTOR = 'iframe[name="biorouter-artifact-preview"]';

/**
 * The height a panel needs to show its content whole: the panel's own chrome
 * (tab strip, borders, the stacked sheet's resize band) plus the content's
 * natural height.
 *
 * Three answers, and the difference between the last two is load-bearing:
 *
 *   - a number — the content said how tall it is: an element marked
 *     `data-preview-intrinsic` whose box is its natural height (a markdown body,
 *     a table, or with `="code"` the `<code>` of a code view), measured inside
 *     its nearest `data-preview-scroller`; or a preview frame that reported its
 *     intrinsic document height (`utils/previewSize.ts`);
 *   - `null` — the content is ready and cannot say (an image, a directory tree,
 *     a live page, a notebook, an error card): the host falls back to half;
 *   - `undefined` — not ready yet (the loading placeholder, a frame that has not
 *     reported). The host keeps waiting; reporting null here would size a fresh
 *     sheet to half and then to its content, the two-step jump this exists to end.
 *
 * Independent of the sheet's height by construction — every term is fixed chrome
 * or a box sized by the content and the panel's WIDTH — so the host resizing the
 * sheet to this number cannot change the number.
 */
export function measurePreviewContentHeight(
  panel: HTMLElement,
  body: HTMLElement,
  frameReport: { source: MessageEvent['source']; height: number } | null
): number | null | undefined {
  // During a switch the outgoing layer is still in the body for its exit; only
  // the current layer says anything about the content's height.
  const content = body.querySelector<HTMLElement>('[data-preview-layer="current"]') ?? body;
  if (content.querySelector('[data-preview-loading]')) return undefined;
  // A directory tree nests a file preview beside its rail; that preview's height
  // says nothing about the tree's.
  if (content.querySelector('[data-preview-opaque]')) return null;
  const panelRect = panel.getBoundingClientRect();
  const bodyRect = body.getBoundingClientRect();
  const chrome = panelRect.height - bodyRect.height;
  const frame = content.querySelector<HTMLIFrameElement>(PREVIEW_FRAME_SELECTOR);
  if (frame) {
    if (!frameReport || !frame.contentWindow || frameReport.source !== frame.contentWindow) {
      return undefined;
    }
    const height = chrome + (frame.getBoundingClientRect().top - bodyRect.top) + frameReport.height;
    return Number.isFinite(height) && height > 0 ? Math.ceil(height) : null;
  }
  const marked = content.querySelector<HTMLElement>('[data-preview-intrinsic]');
  if (!marked) return null;
  // `="code"`: the highlighter's block is `min-height: 100%` of the scroller, so
  // ITS height follows the sheet and would only ever ratchet up. The `<code>`
  // inside it is sized by the text alone.
  const target =
    marked.getAttribute('data-preview-intrinsic') === 'code'
      ? marked.querySelector<HTMLElement>('code')
      : marked;
  if (!target) return null;
  const scroller = marked.closest<HTMLElement>('[data-preview-scroller]') ?? content;
  let below = 0;
  for (let el = target.parentElement; el && el !== scroller; el = el.parentElement) {
    const cs = getComputedStyle(el);
    below +=
      (Number.parseFloat(cs.paddingBottom) || 0) + (Number.parseFloat(cs.borderBottomWidth) || 0);
  }
  const scrollerRect = scroller.getBoundingClientRect();
  const style = getComputedStyle(scroller);
  const horizontalScrollbar = Math.max(
    0,
    scroller.offsetHeight -
      scroller.clientHeight -
      (Number.parseFloat(style.borderTopWidth) || 0) -
      (Number.parseFloat(style.borderBottomWidth) || 0)
  );
  const contentBottom =
    target.getBoundingClientRect().bottom -
    scrollerRect.top +
    scroller.scrollTop +
    below +
    (Number.parseFloat(style.paddingBottom) || 0) +
    horizontalScrollbar;
  const height = chrome + (scrollerRect.top - bodyRect.top) + contentBottom;
  return Number.isFinite(height) && height > 0 ? Math.ceil(height) : null;
}

export interface ArtifactRenderError {
  artifactTitle: string;
  message: string;
  detail?: string;
  href?: string;
}

type HtmlPreview = { kind: 'html'; html: string };
type ExternalPreview = { kind: 'externalUrl'; url: string; managedApp: boolean };
type FilePreview = { kind: 'file'; preview: ArtifactFilePreview };
type McpPreview = { kind: 'mcpResource' };
type LoadingPreview = { kind: 'loading' };
type ErrorPreview = { kind: 'error'; message: string };
type PreviewState =
  | HtmlPreview
  | ExternalPreview
  | FilePreview
  | McpPreview
  | LoadingPreview
  | ErrorPreview;

const LOADING_PREVIEW: LoadingPreview = { kind: 'loading' };

/** A live page paints a native view that cannot fade, so it never crossfades. */
function isLivePagePreview(preview: PreviewState, browsing: boolean) {
  return preview.kind === 'externalUrl' && (preview.managedApp || browsing);
}

/**
 * Whether this engine can run the panel's layer crossfade. jsdom has no Web
 * Animations API and runs no CSS, so it gets the plain swap: a test then sees
 * exactly one layer, never one that is fading out.
 */
function canCrossfadeLayers() {
  return (
    typeof Element !== 'undefined' &&
    typeof Element.prototype.animate === 'function' &&
    !prefersReducedMotion()
  );
}

/** A layer that is fading out after a switch: what it showed, kept for one exit. */
type OutgoingLayer = { key: string; preview: PreviewState; artifact: ArtifactSource };

type ArtifactTab = {
  id: string;
  artifact: ArtifactSource;
};

type ArtifactTabsState = {
  tabs: ArtifactTab[];
  activeTabId: string | null;
};

type ArtifactTabsAction =
  | { type: 'open'; artifact: ArtifactSource; newTabId: string }
  | { type: 'activate'; tabId: string }
  | { type: 'close'; tabId: string }
  | { type: 'reorder'; draggedTabId: string; targetTabId: string };

let artifactTabSequence = 0;

function nextArtifactTabId() {
  artifactTabSequence += 1;
  return `artifact-tab-${artifactTabSequence}`;
}

function artifactSourceKey(artifact: ArtifactSource) {
  if (artifact.kind === 'file') return `file:${artifact.path}`;
  if (artifact.kind === 'externalUrl') return `url:${artifact.url}`;
  if (artifact.kind === 'html') {
    return `html:${artifact.title}:${artifact.html.length}:${artifact.html.slice(0, 80)}`;
  }
  const resource = artifact.resource as {
    uri?: string;
    mimeType?: string;
    text?: string;
    blob?: string;
  };
  return `resource:${resource.uri ?? ''}:${resource.mimeType ?? ''}:${resource.text?.length ?? 0}:${resource.blob?.length ?? 0}`;
}

function artifactHoverTitle(artifact: ArtifactSource) {
  if (artifact.kind === 'file') return artifact.path;
  if (artifact.kind === 'externalUrl') return artifact.url;
  if (artifact.kind === 'mcpResource') return artifact.resource.uri;
  return artifact.title;
}

function createArtifactTabsState(artifact: ArtifactSource | null): ArtifactTabsState {
  if (!artifact) return { tabs: [], activeTabId: null };
  const id = nextArtifactTabId();
  return { tabs: [{ id, artifact }], activeTabId: id };
}

function artifactTabsReducer(
  state: ArtifactTabsState,
  action: ArtifactTabsAction
): ArtifactTabsState {
  if (action.type === 'open') {
    const key = artifactSourceKey(action.artifact);
    const existing = state.tabs.find((tab) => artifactSourceKey(tab.artifact) === key);
    if (existing) {
      return {
        tabs: state.tabs.map((tab) =>
          tab.id === existing.id ? { ...tab, artifact: action.artifact } : tab
        ),
        activeTabId: existing.id,
      };
    }
    return {
      tabs: [...state.tabs, { id: action.newTabId, artifact: action.artifact }],
      activeTabId: action.newTabId,
    };
  }

  if (action.type === 'activate') return { ...state, activeTabId: action.tabId };

  if (action.type === 'reorder') {
    const draggedIndex = state.tabs.findIndex((tab) => tab.id === action.draggedTabId);
    const targetIndex = state.tabs.findIndex((tab) => tab.id === action.targetTabId);
    if (draggedIndex < 0 || targetIndex < 0 || draggedIndex === targetIndex) return state;

    const tabs = [...state.tabs];
    const [draggedTab] = tabs.splice(draggedIndex, 1);
    tabs.splice(targetIndex, 0, draggedTab);
    return { ...state, tabs };
  }

  const closingIndex = state.tabs.findIndex((tab) => tab.id === action.tabId);
  if (closingIndex < 0) return state;
  const tabs = state.tabs.filter((tab) => tab.id !== action.tabId);
  if (state.activeTabId !== action.tabId) return { ...state, tabs };
  const nextTab = tabs[Math.min(closingIndex, tabs.length - 1)] ?? null;
  return { tabs, activeTabId: nextTab?.id ?? null };
}

// The artifact preview shares the chat renderer's palette rather than maintaining
// a second, divergent one. Both come from styles/codeTheme.ts (design.md §5.1),
// selected by the active theme family + mode via codeThemesByFamily.

/// A render-error field is forwarded into an agent prompt, so bound its length
/// rather than letting a figure paste an arbitrarily long payload into context.
const RENDER_ERROR_TEXT_LIMIT = 2000;

function clampRenderErrorText(value: string) {
  return value.length > RENDER_ERROR_TEXT_LIMIT
    ? `${value.slice(0, RENDER_ERROR_TEXT_LIMIT)}…`
    : value;
}

function iconForArtifact(artifact: ArtifactSource | null) {
  if (!artifact) return File;
  if (artifact.kind === 'externalUrl') return Globe;
  if (artifact.kind === 'file') {
    const ext = artifact.path.split('.').pop()?.toLowerCase();
    if (isImageExtension(ext)) return Image;
    if (['html', 'htm'].includes(ext || '')) return Globe;
    if (['js', 'ts', 'tsx', 'jsx', 'py', 'rs', 'sql', 'json', 'yaml', 'yml'].includes(ext || '')) {
      return Code;
    }
    return FileText;
  }
  // A `ui://` figure or report (Auto Visualiser, an app card): the figure glyph
  // the tool row that made it draws (icons.md, "Figure").
  return ChartColumn;
}

function formatBytes(value?: number) {
  if (value === undefined) return '';
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export default function ArtifactViewer({
  artifact,
  isOpen = true,
  motionReady = true,
  isResizing = false,
  onClose,
  onOpenArtifact,
  onResizeStart,
  onRenderError,
  onLiveBrowserShareChange,
  onFilePreviewRevisionChange,
  sessionId,
  refreshRevision = 0,
  className,
  style,
  layout = 'side',
  folded = false,
  onToggleFold,
  onUnfold,
  onContentHeightChange,
}: ArtifactViewerProps) {
  const { resolvedTheme } = useTheme();
  const [tabState, dispatchTabAction] = useReducer(
    artifactTabsReducer,
    artifact,
    createArtifactTabsState
  );
  const [preview, setPreview] = useState<PreviewState>(LOADING_PREVIEW);
  const [previewSourceKey, setPreviewSourceKey] = useState<string | null>(null);
  // The refresh scope (session + source) `preview` was loaded for, as state, so
  // the render can tell a stale scope's preview from this one's.
  const [previewScope, setPreviewScope] = useState('');
  const previewRef = useRef(preview);
  previewRef.current = preview;
  // The refresh scope (session + source) the `preview` state belongs to.
  const loadedScopeRef = useRef('');
  /**
   * STALE-WHILE-LOADING (spec 3.8). The last preview each tab showed, keyed by
   * its refresh scope. Activating a tab renders this at once and refreshes it in
   * the background, so a tab that has been seen never flashes a loading state
   * again — the file is re-read, and only a CHANGED revision re-renders.
   */
  const previewCacheRef = useRef(new Map<string, PreviewState>());
  const refreshTrackerRef = useRef({ scope: '', revision: refreshRevision });
  const [fileRefresh, setFileRefresh] = useState({ scope: '', revision: 0 });
  const [updateReady, setUpdateReady] = useState(false);
  const [refreshFailed, setRefreshFailed] = useState(false);
  const deferredRefreshRef = useRef(false);
  const htmlDirtyRef = useRef(false);
  const manualRefreshRef = useRef(false);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [dragOverTabId, setDragOverTabId] = useState<string | null>(null);
  // Which ends of the tab list are scrolled past, for the edge fade.
  const [tabListScrolled, setTabListScrolled] = useState({ start: false, end: false });
  const lastRenderErrorKeyRef = useRef<string | null>(null);
  const trustedFrameRef = useRef<HTMLIFrameElement | null>(null);
  // A fading layer's frame is never the trusted one: render errors come only
  // from the frame the person is looking at.
  const outgoingFrameRef = useRef<HTMLIFrameElement | null>(null);
  /**
   * URLs the user has explicitly chosen to browse inside the panel.
   *
   * Kept as *opt-in state* rather than a property of the artifact, because that
   * is what makes agent-initiated navigation impossible: an `externalUrl`
   * artifact can arrive from an MCP resource link with no transcript card and
   * auto-open, so the live view has to be reachable only through a click that a
   * person made. Scoped to this panel instance and deliberately not persisted —
   * reopening a session should not silently start loading pages.
   */
  const [browsingUrls, setBrowsingUrls] = useState<ReadonlySet<string>>(() => new Set());
  const [isAnnotating, setIsAnnotating] = useState(false);
  const interactionRef = useRef({ isAnnotating, isResizing });
  interactionRef.current = { isAnnotating, isResizing };
  const [liveBrowserViewId, setLiveBrowserViewId] = useState<string | null>(null);
  const liveBrowserViewIdRef = useRef(liveBrowserViewId);
  const handleLiveBrowserViewChange = useCallback((viewId: string | null) => {
    liveBrowserViewIdRef.current = viewId;
    setLiveBrowserViewId(viewId);
  }, []);
  const [annotationSnapshot, setAnnotationSnapshot] = useState<{
    path: string;
    dataUrl: string;
    sourceTitle: string;
    sourceUrl: string;
    sourceRevision: string;
  } | null>(null);
  const annotationSnapshotRef = useRef(annotationSnapshot);
  annotationSnapshotRef.current = annotationSnapshot;
  const previewBodyRef = useRef<HTMLDivElement | null>(null);
  usePreviewMotion(previewBodyRef, { isOpen, layout, ready: motionReady });
  // The preview the tabs name in `aria-controls`. Per panel, never a literal:
  // the document resolves a shared id to its first holder, which is how every
  // composer's Send came to submit the left pane's form in a split. Only the
  // active group's panel renders today (`artifactPanelEnabled={isActiveGroup}`
  // in ChatGroupsShell), so two panels do not coexist in the running app — but
  // nothing about this component requires that, and a literal id here would
  // quietly point a second panel's tabs at the first one's preview.
  // `ArtifactViewer.splitPane.test.tsx`.
  const previewContentId = useId();
  const annotateReasonId = useId();
  const activeSourceKeyRef = useRef<string | null>(null);
  const activeSourceGenerationRef = useRef(0);

  const pendingNavigationKeyRef = useRef<string | null>(null);
  const draggedTabIdRef = useRef<string | null>(null);
  const tabPointerGestureRef = useRef<{
    tabId: string;
    startX: number;
    startY: number;
  } | null>(null);
  const dragOverTabIdRef = useRef<string | null>(null);
  const suppressTabClickRef = useRef(false);
  const suppressTabClickTimerRef = useRef<number | null>(null);
  const activeTabButtonRef = useRef<HTMLButtonElement | null>(null);
  const tabListRef = useRef<HTMLDivElement | null>(null);
  /**
   * Rung 3 (D-32) for the panel's strip, through the SAME rule the chat strip
   * uses — card Z's "one tab, three surfaces" is only true if the rule is shared
   * too. Called up here with the other hooks: the component early-returns below
   * when there is no active artifact.
   */
  const showTabOverflowMenu = useTabStripOverflow(tabListRef, tabState.tabs.length);
  const hasTabs = tabState.tabs.length > 0;
  const activeTab = tabState.tabs.find((tab) => tab.id === tabState.activeTabId) ?? null;
  const activeArtifact = activeTab?.artifact ?? null;
  const quotedRevision =
    preview.kind === 'file' && 'revision' in preview.preview ? preview.preview.revision : undefined;
  const quotedSelection = useTextSelection(
    previewBodyRef,
    `${sessionId}:${isOpen}:${activeTab?.id ?? ''}:${preview.kind}:${previewSourceKey}:${quotedRevision ?? ''}`
  );
  const quoteSource = {
    sessionId: sessionId ?? '',
    title: activeArtifact?.title ?? 'Preview',
    locator:
      activeArtifact?.kind === 'file'
        ? activeArtifact.path
        : activeArtifact?.kind === 'externalUrl'
          ? activeArtifact.url
          : undefined,
    revision: quotedRevision,
  };

  /**
   * Turn a selected region into an attachment on the composer.
   *
   * The rectangle is in the preview body's own coordinates; `capturePage`
   * wants page coordinates, so the body's position is added back. The capture
   * is a compositor grab in the main process, which is the only thing that can
   * see into the sandboxed `srcdoc` frames most artifacts render in — a
   * DOM-walking screenshot library would return an empty box for a figure.
   */
  const finishAnnotation = useCallback(() => {
    setIsAnnotating(false);
    setAnnotationSnapshot((current) => {
      if (current) window.electron?.deleteTempFile(current.path);
      return null;
    });
  }, []);

  const captureAnnotation = useCallback(
    async (region: SelectedRegion) => {
      const body = previewBodyRef.current;
      if (!body || !sessionId) {
        finishAnnotation();
        return;
      }
      const sourceKey = activeSourceKeyRef.current;
      const sourceGeneration = activeSourceGenerationRef.current;
      const liveSnapshot = activeArtifact?.kind === 'externalUrl' ? annotationSnapshot : null;
      const bodyRect = body.getBoundingClientRect();
      const hasMeasuredBounds = bodyRect.width > 0 && bodyRect.height > 0;
      const x = hasMeasuredBounds ? Math.min(bodyRect.width, Math.max(0, region.x)) : region.x;
      const y = hasMeasuredBounds ? Math.min(bodyRect.height, Math.max(0, region.y)) : region.y;
      const width = hasMeasuredBounds ? Math.min(region.width, bodyRect.width - x) : region.width;
      const height = hasMeasuredBounds
        ? Math.min(region.height, bodyRect.height - y)
        : region.height;
      if (width <= 0 || height <= 0) {
        finishAnnotation();
        return;
      }
      const shot = await window.electron?.captureRegion?.({
        x: bodyRect.left + x,
        y: bodyRect.top + y,
        width,
        height,
        label: 'annotation',
        ...(hasMeasuredBounds
          ? {
              containment: {
                x: bodyRect.left,
                y: bodyRect.top,
                width: bodyRect.width,
                height: bodyRect.height,
              },
            }
          : {}),
      });
      if (!shot) {
        finishAnnotation();
        return;
      }
      if (
        !sourceKey ||
        sourceKey !== activeSourceKeyRef.current ||
        sourceGeneration !== activeSourceGenerationRef.current
      ) {
        window.electron?.deleteTempFile(shot.path);
        finishAnnotation();
        return;
      }
      sendArtifactAnnotation({
        sessionId,
        imagePath: shot.path,
        sourceTitle: liveSnapshot?.sourceTitle || activeArtifact?.title || 'Preview',
        sourceLocator:
          liveSnapshot?.sourceUrl ??
          (activeArtifact?.kind === 'file'
            ? activeArtifact.path
            : activeArtifact?.kind === 'externalUrl'
              ? activeArtifact.url
              : undefined),
        sourceRevision:
          liveSnapshot?.sourceRevision ??
          (preview.kind === 'file' &&
          previewSourceKey === activeSourceKeyRef.current &&
          'revision' in preview.preview
            ? preview.preview.revision
            : undefined),
        sourceTrust:
          liveSnapshot || activeArtifact?.kind === 'externalUrl' ? 'untrusted_external' : 'local',
        region: {
          x,
          y,
          width,
          height,
          surfaceWidth: bodyRect.width,
          surfaceHeight: bodyRect.height,
        },
        width,
        height,
      });
      finishAnnotation();
    },
    [activeArtifact, annotationSnapshot, finishAnnotation, preview, previewSourceKey, sessionId]
  );

  const activeSourceKey = activeArtifact ? artifactSourceKey(activeArtifact) : null;
  activeSourceKeyRef.current = activeSourceKey;

  useEffect(() => {
    activeSourceGenerationRef.current += 1;
    finishAnnotation();
    handleLiveBrowserViewChange(null);
  }, [activeSourceKey, finishAnnotation, handleLiveBrowserViewChange]);

  useEffect(
    () => () => {
      activeSourceGenerationRef.current += 1;
      activeSourceKeyRef.current = null;
      liveBrowserViewIdRef.current = null;
      const snapshot = annotationSnapshotRef.current;
      if (snapshot) window.electron?.deleteTempFile(snapshot.path);
      onLiveBrowserShareChange?.(null);
    },
    [onLiveBrowserShareChange]
  );

  const toggleAnnotation = useCallback(async () => {
    if (isAnnotating) {
      finishAnnotation();
      return;
    }
    if (activeArtifact?.kind !== 'externalUrl') {
      setIsAnnotating(true);
      return;
    }

    const sourceKey = activeSourceKeyRef.current;
    const sourceGeneration = activeSourceGenerationRef.current;
    const viewId = liveBrowserViewId;
    if (!viewId) return;
    const shot = await window.electron?.embeddedBrowser?.capture(viewId);
    if (!shot) return;

    let viewHidden = false;
    const rejectCapture = () => {
      window.electron?.deleteTempFile(shot.path);
      if (viewHidden) void window.electron?.embeddedBrowser?.setVisible(viewId, true);
    };
    const sourceIsCurrent = () =>
      sourceKey !== null &&
      sourceKey === activeSourceKeyRef.current &&
      sourceGeneration === activeSourceGenerationRef.current &&
      liveBrowserViewIdRef.current === viewId;
    const pageIsCurrent = async (expected?: { url: string; sourceRevision: string }) => {
      if (!sourceIsCurrent()) return null;
      const page = await window.electron?.embeddedBrowser?.readText(viewId, 0);
      if (
        !page ||
        !sourceIsCurrent() ||
        shot.sourceRevision !== page.sourceRevision ||
        (expected && (page.url !== expected.url || page.sourceRevision !== expected.sourceRevision))
      ) {
        return null;
      }
      return page;
    };

    try {
      const capturedPage = await pageIsCurrent();
      if (!capturedPage) {
        rejectCapture();
        return;
      }
      const dataUrl = await window.electron?.getTempImage(shot.path);
      if (!dataUrl || !(await pageIsCurrent(capturedPage))) {
        rejectCapture();
        return;
      }
      await window.electron?.embeddedBrowser?.setVisible(viewId, false);
      viewHidden = true;
      if (!(await pageIsCurrent(capturedPage))) {
        rejectCapture();
        return;
      }
      setAnnotationSnapshot({
        path: shot.path,
        dataUrl,
        sourceTitle: capturedPage.title || activeArtifact.title,
        sourceUrl: capturedPage.url,
        sourceRevision: capturedPage.sourceRevision,
      });
      setIsAnnotating(true);
    } catch {
      rejectCapture();
    }
  }, [activeArtifact, finishAnnotation, isAnnotating, liveBrowserViewId]);

  const openArtifactInTab = useCallback(
    (nextArtifact: ArtifactSource) => {
      pendingNavigationKeyRef.current = artifactSourceKey(nextArtifact);
      dispatchTabAction({
        type: 'open',
        artifact: nextArtifact,
        newTabId: nextArtifactTabId(),
      });
      onOpenArtifact(nextArtifact);
    },
    [onOpenArtifact]
  );

  const closeTab = useCallback(
    (tabId: string) => {
      if (tabState.tabs.length === 1) {
        onClose();
        return;
      }

      const closingIndex = tabState.tabs.findIndex((tab) => tab.id === tabId);
      const closingActiveTab = tabState.activeTabId === tabId;
      const remainingTabs = tabState.tabs.filter((tab) => tab.id !== tabId);
      dispatchTabAction({ type: 'close', tabId });

      if (closingActiveTab) {
        const nextTab = remainingTabs[Math.min(closingIndex, remainingTabs.length - 1)];
        if (nextTab) {
          pendingNavigationKeyRef.current = artifactSourceKey(nextTab.artifact);
          onOpenArtifact(nextTab.artifact);
        }
      }
    },
    [onClose, onOpenArtifact, tabState]
  );

  useEffect(() => {
    const handleBrowserTabShortcuts = (event: KeyboardEvent) => {
      if (!isOpen || !tabState.activeTabId) return;

      if (
        event.key.toLowerCase() === 'w' &&
        (event.metaKey || event.ctrlKey) &&
        !event.altKey &&
        !event.shiftKey
      ) {
        event.preventDefault();
        event.stopPropagation();
        closeTab(tabState.activeTabId);
        return;
      }

      if (!isTabCycleEvent(event)) return;

      // Ctrl+Tab belongs to whichever strip has focus. Without this the panel
      // cycled previews from ANYWHERE the moment it was open — including with
      // the cursor in the composer, where the user means their chat tabs. The
      // chat strip consults the same predicate and takes the other branch, so
      // the two cannot both answer regardless of listener order.
      if (!isWithinArtifactPanel(event.target)) return;

      const activeIndex = tabState.tabs.findIndex((tab) => tab.id === tabState.activeTabId);
      const nextIndex = nextTabIndex(tabState.tabs.length, activeIndex, tabCycleOffset(event));
      if (nextIndex === null) return;

      event.preventDefault();
      event.stopPropagation();
      const nextTab = tabState.tabs[nextIndex];
      pendingNavigationKeyRef.current = artifactSourceKey(nextTab.artifact);
      dispatchTabAction({ type: 'activate', tabId: nextTab.id });
      onOpenArtifact(nextTab.artifact);
    };

    window.addEventListener('keydown', handleBrowserTabShortcuts, true);
    return () => window.removeEventListener('keydown', handleBrowserTabShortcuts, true);
  }, [closeTab, isOpen, onOpenArtifact, tabState.activeTabId, tabState.tabs]);

  /**
   * Keep the active tab whole inside the tab list (P0 1).
   *
   * It runs when the active tab or the tab count changes, when the ▾ overflow
   * button mounts (it narrows the list one render AFTER the overflow is
   * measured, which is how the active tab used to end up clipped behind it), and
   * whenever the list itself is resized. It scrolls the LIST, never
   * `scrollIntoView`, so no ancestor (the split box, the chat column) can be
   * scrolled along with it. Instant on a resize; smooth on a switch unless the
   * person asked for less motion.
   */
  const revealActiveTab = useCallback((behavior: 'auto' | 'smooth') => {
    const list = tabListRef.current;
    const tab = activeTabButtonRef.current?.closest<HTMLElement>('.br-tab');
    if (!list || !tab) return;
    const pad = 8;
    const listRect = list.getBoundingClientRect();
    const tabRect = tab.getBoundingClientRect();
    // Unlaid-out (a hidden group, jsdom): nothing to reveal against.
    if (listRect.width === 0 || tabRect.width === 0) return;
    let delta = 0;
    if (tabRect.left < listRect.left + pad) delta = tabRect.left - listRect.left - pad;
    else if (tabRect.right > listRect.right - pad) delta = tabRect.right - listRect.right + pad;
    if (Math.abs(delta) < 1) return;
    const left = Math.max(0, list.scrollLeft + delta);
    if (typeof list.scrollTo === 'function') list.scrollTo({ left, behavior });
    else list.scrollLeft = left;
  }, []);

  useEffect(() => {
    revealActiveTab(prefersReducedMotion() ? 'auto' : 'smooth');
  }, [revealActiveTab, showTabOverflowMenu, tabState.activeTabId, tabState.tabs.length]);

  useEffect(() => {
    const list = tabListRef.current;
    if (!list) return;
    const syncScrolled = () => {
      const start = list.scrollLeft > 1;
      const end = list.scrollLeft + list.clientWidth < list.scrollWidth - 1;
      setTabListScrolled((current) =>
        current.start === start && current.end === end ? current : { start, end }
      );
    };
    syncScrolled();
    list.addEventListener('scroll', syncScrolled, { passive: true });
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(() => {
            syncScrolled();
            revealActiveTab('auto');
          });
    observer?.observe(list);
    return () => {
      list.removeEventListener('scroll', syncScrolled);
      observer?.disconnect();
    };
    // Re-attached when the list mounts (the panel early-returns with no tab).
  }, [revealActiveTab, hasTabs]);

  useEffect(() => {
    const moveTabPointer = (event: globalThis.PointerEvent) => {
      const gesture = tabPointerGestureRef.current;
      if (!gesture) return;

      if (!draggedTabIdRef.current) {
        const distance = Math.hypot(event.clientX - gesture.startX, event.clientY - gesture.startY);
        if (distance < 5) return;
        draggedTabIdRef.current = gesture.tabId;
        suppressTabClickRef.current = true;
        setDraggedTabId(gesture.tabId);
      }

      event.preventDefault();
      const target = document
        .elementFromPoint(event.clientX, event.clientY)
        ?.closest<HTMLElement>('[data-artifact-tab-id]');
      const targetTabId = target?.dataset.artifactTabId ?? null;
      const nextDragOverTabId = targetTabId === gesture.tabId ? null : targetTabId;
      dragOverTabIdRef.current = nextDragOverTabId;
      setDragOverTabId(nextDragOverTabId);
    };

    const finishTabPointer = () => {
      const sourceTabId = draggedTabIdRef.current;
      const targetTabId = dragOverTabIdRef.current;
      if (sourceTabId && targetTabId) {
        dispatchTabAction({ type: 'reorder', draggedTabId: sourceTabId, targetTabId });
      }
      window.clearTimeout(suppressTabClickTimerRef.current ?? undefined);
      suppressTabClickTimerRef.current = window.setTimeout(() => {
        suppressTabClickRef.current = false;
        suppressTabClickTimerRef.current = null;
      }, 0);
      tabPointerGestureRef.current = null;
      draggedTabIdRef.current = null;
      dragOverTabIdRef.current = null;
      setDraggedTabId(null);
      setDragOverTabId(null);
    };

    window.addEventListener('pointermove', moveTabPointer);
    window.addEventListener('pointerup', finishTabPointer);
    window.addEventListener('pointercancel', finishTabPointer);
    return () => {
      window.removeEventListener('pointermove', moveTabPointer);
      window.removeEventListener('pointerup', finishTabPointer);
      window.removeEventListener('pointercancel', finishTabPointer);
      window.clearTimeout(suppressTabClickTimerRef.current ?? undefined);
    };
  }, []);

  useEffect(() => {
    if (!artifact) return;
    const key = artifactSourceKey(artifact);
    if (pendingNavigationKeyRef.current === key) {
      pendingNavigationKeyRef.current = null;
      return;
    }
    dispatchTabAction({ type: 'open', artifact, newTabId: nextArtifactTabId() });
  }, [artifact]);

  const refreshScope = JSON.stringify([sessionId, activeSourceKey]);
  const fileRefreshRevision = fileRefresh.scope === refreshScope ? fileRefresh.revision : 0;

  useEffect(() => {
    const tracker = refreshTrackerRef.current;
    if (tracker.scope !== refreshScope) {
      refreshTrackerRef.current = { scope: refreshScope, revision: refreshRevision };
      setUpdateReady(false);
      setRefreshFailed(false);
      deferredRefreshRef.current = false;
      htmlDirtyRef.current = false;
      return;
    }
    if (
      activeArtifact?.kind !== 'file' ||
      (tracker.revision === refreshRevision && !deferredRefreshRef.current)
    )
      return;
    const focused = document.activeElement;
    const editing =
      focused instanceof HTMLElement &&
      previewBodyRef.current?.contains(focused) &&
      (focused.matches('input, textarea, select, iframe, [role="textbox"]') ||
        focused.isContentEditable);
    if (isAnnotating || isResizing || editing || htmlDirtyRef.current) {
      setUpdateReady(true);
      return;
    }
    tracker.revision = refreshRevision;
    deferredRefreshRef.current = false;
    setUpdateReady(false);
    setFileRefresh((previous) => ({ scope: refreshScope, revision: previous.revision + 1 }));
  }, [activeArtifact?.kind, isAnnotating, isResizing, refreshRevision, refreshScope]);

  useEffect(() => {
    let cancelled = false;
    const manualRefresh = manualRefreshRef.current;
    manualRefreshRef.current = false;
    // What this scope showed last: the live state when it is still this scope's,
    // otherwise the cache a tab switch left behind.
    const cached = previewCacheRef.current.get(refreshScope);
    const previous =
      loadedScopeRef.current === refreshScope ? previewRef.current : (cached ?? LOADING_PREVIEW);
    const retainPrevious = previous.kind === 'file' && previous.preview.found;

    /** Show `next` for this scope and remember it for the next activation. */
    const show = (next: PreviewState) => {
      previewCacheRef.current.set(refreshScope, next);
      loadedScopeRef.current = refreshScope;
      setPreview(next);
    };

    const publishFile = (response: ArtifactFilePreview) => {
      if (cancelled) return;
      const focused = document.activeElement;
      const editing =
        focused instanceof HTMLElement &&
        previewBodyRef.current?.contains(focused) &&
        (focused.matches('input, textarea, select, iframe, [role="textbox"]') ||
          focused.isContentEditable);
      if (
        retainPrevious &&
        (interactionRef.current.isAnnotating ||
          interactionRef.current.isResizing ||
          editing ||
          (htmlDirtyRef.current && !manualRefresh))
      ) {
        deferredRefreshRef.current = true;
        setUpdateReady(true);
        return;
      }
      if (!response.found && retainPrevious) {
        setRefreshFailed(true);
        return;
      }
      setRefreshFailed(false);
      if (
        retainPrevious &&
        previous.kind === 'file' &&
        'revision' in response &&
        response.revision &&
        'revision' in previous.preview &&
        response.revision === previous.preview.revision &&
        (response.kind !== 'html' ||
          (previous.preview.kind === 'html' &&
            response.preparedHtml === previous.preview.preparedHtml))
      )
        return;
      htmlDirtyRef.current = false;
      show({ kind: 'file', preview: response });
    };

    async function loadPreview() {
      if (!activeArtifact || !activeSourceKey) return;
      setPreviewSourceKey(activeSourceKey);
      setPreviewScope(refreshScope);
      if (loadedScopeRef.current !== refreshScope) {
        // A tab that has been seen shows its last preview at once (and never a
        // loading state); a first visit shows the quiet skeleton.
        loadedScopeRef.current = refreshScope;
        setPreview(cached ?? LOADING_PREVIEW);
      } else if (!retainPrevious && previous.kind !== 'loading' && !cached) {
        setPreview(LOADING_PREVIEW);
      }

      if (activeArtifact.kind === 'html') {
        try {
          const prepared = await window.electron.prepareArtifactHtml({
            html: activeArtifact.html,
          });
          if (!cancelled) show({ kind: 'html', html: prepared.html });
        } catch {
          if (!cancelled) show({ kind: 'html', html: activeArtifact.html });
        }
        return;
      }

      if (activeArtifact.kind === 'externalUrl') {
        let managedApp = false;
        try {
          managedApp =
            (await window.electron.embeddedBrowser.isManagedAppUrl?.(activeArtifact.url)) === true;
        } catch {
          managedApp = false;
        }
        if (!cancelled) {
          const current = previewCacheRef.current.get(refreshScope);
          // Same answer as last time: keep the object, so nothing re-renders.
          if (
            current?.kind !== 'externalUrl' ||
            current.url !== activeArtifact.url ||
            current.managedApp !== managedApp
          ) {
            show({ kind: 'externalUrl', url: activeArtifact.url, managedApp });
          }
        }
        return;
      }

      if (activeArtifact.kind === 'mcpResource') {
        show({ kind: 'mcpResource' });
        return;
      }

      try {
        // No cast: the preload IPC contract and the shared ArtifactFilePreview
        // union are kept in lockstep, so the result assigns structurally.
        const response: ArtifactFilePreview = await window.electron.readArtifactFile(
          activeArtifact.path
        );
        if (cancelled) return;
        if (response.kind === 'html') {
          // An HTML file the agent wrote gets a Preview/Raw toggle, like markdown:
          // keep it a file preview (so `text` remains the raw source) and attach a
          // security-prepared copy for the rendered Preview. A `ui://` figure
          // resource is `artifact.kind === 'html'` and took the branch above, so it
          // still renders figure-only with no raw source — only real files land here.
          let preparedHtml = response.text;
          try {
            const prepared = await window.electron.prepareArtifactHtml({
              html: response.text,
            });
            preparedHtml = prepared.html;
          } catch {
            if (retainPrevious) {
              if (!cancelled) setRefreshFailed(true);
              return;
            }
            // Preparation failed; fall back to the raw HTML for the Preview.
          }
          publishFile({ ...response, preparedHtml });
          return;
        }
        publishFile(response);
      } catch (error) {
        if (!cancelled) {
          if (retainPrevious) setRefreshFailed(true);
          else
            show({
              kind: 'error',
              message: error instanceof Error ? error.message : 'Could not open artifact.',
            });
        }
      }
    }

    loadPreview();
    return () => {
      cancelled = true;
    };
  }, [activeArtifact, activeSourceKey, fileRefreshRevision, refreshScope]);

  useEffect(() => {
    if (activeArtifact?.kind !== 'file') return;
    const trackDirty = (event: MessageEvent) => {
      if (event.data?.type !== 'biorouter-artifact-dirty') return;
      const frames = previewBodyRef.current?.querySelectorAll(
        'iframe[name="biorouter-artifact-preview"]'
      );
      if (
        frames &&
        [...frames].some((frame) => (frame as HTMLIFrameElement).contentWindow === event.source)
      ) {
        htmlDirtyRef.current = true;
      }
    };
    window.addEventListener('message', trackDirty);
    return () => window.removeEventListener('message', trackDirty);
  }, [activeArtifact?.kind, refreshScope]);

  useEffect(() => {
    const revision =
      preview.kind === 'file' && previewSourceKey === activeSourceKey
        ? 'revision' in preview.preview
          ? preview.preview.revision || null
          : null
        : null;
    onFilePreviewRevisionChange?.(revision);
  }, [activeSourceKey, onFilePreviewRevisionChange, preview, previewSourceKey]);

  useEffect(
    () => () => {
      onFilePreviewRevisionChange?.(null);
    },
    [onFilePreviewRevisionChange]
  );

  useEffect(() => {
    if (!activeArtifact || !onRenderError) return;

    const handleMessage = (event: MessageEvent) => {
      // A render-error report becomes a hidden, agent-visible prompt. Only the
      // srcDoc frame we generated may send one: an externalUrl artifact, an
      // mcp-ui frame, or any other window would otherwise be able to inject
      // instructions into a session that holds shell and file tools.
      const trustedSource = trustedFrameRef.current?.contentWindow;
      if (!trustedSource || event.source !== trustedSource) return;

      const data = event.data as
        | {
            type?: string;
            payload?: {
              message?: unknown;
              detail?: unknown;
              href?: unknown;
            };
          }
        | undefined;

      if (!data || data.type !== 'biorouter-viz-render-error') return;

      const message =
        typeof data.payload?.message === 'string'
          ? clampRenderErrorText(data.payload.message)
          : 'This visualization could not be rendered.';
      const detail =
        typeof data.payload?.detail === 'string'
          ? clampRenderErrorText(data.payload.detail)
          : undefined;
      const href =
        typeof data.payload?.href === 'string'
          ? clampRenderErrorText(data.payload.href)
          : undefined;
      const key = `${activeArtifact.title}\n${message}\n${detail ?? ''}`;
      if (lastRenderErrorKeyRef.current === key) return;
      lastRenderErrorKeyRef.current = key;

      onRenderError({
        artifactTitle: activeArtifact.title,
        message,
        detail,
        href,
      });
    };

    window.addEventListener('message', handleMessage);
    return () => window.removeEventListener('message', handleMessage);
  }, [activeArtifact, onRenderError]);

  // The preview for the ACTIVE tab: the live state when it was loaded for this
  // scope, else what this tab showed last (stale-while-loading), else nothing yet.
  const visiblePreview: PreviewState = !activeSourceKey
    ? LOADING_PREVIEW
    : previewScope === refreshScope && previewSourceKey === activeSourceKey
      ? preview
      : (previewCacheRef.current.get(refreshScope) ?? LOADING_PREVIEW);

  /**
   * THE SWITCH (spec 3.8). Each source renders in its own layer, keyed by that
   * source, so a switch mounts the incoming tab and keeps the outgoing tab's DOM
   * — not a re-render of it, its own instance — for one 125ms exit while the
   * incoming layer rises 8px and fades in over 175ms. Derived during render
   * (React's "adjust state when a prop changes"), so the outgoing layer is part
   * of the very commit that switches: there is no frame with neither.
   *
   * The first content a panel shows does NOT animate: the body's own entrance
   * (`usePreviewMotion`) already brings it in. A live page never crossfades — its
   * native view cannot fade — and nothing crossfades without a real engine or
   * under reduced motion.
   */
  const shownLayerRef = useRef<OutgoingLayer | null>(null);
  const contentShownRef = useRef(false);
  const [layers, setLayers] = useState<{
    key: string | null;
    animate: boolean;
    outgoing: OutgoingLayer | null;
  }>(() => ({ key: activeSourceKey, animate: false, outgoing: null }));
  if (layers.key !== activeSourceKey) {
    const shown = shownLayerRef.current;
    const outgoing =
      shown &&
      shown.key === layers.key &&
      shown.preview.kind !== 'loading' &&
      !isLivePagePreview(
        shown.preview,
        shown.preview.kind === 'externalUrl' && browsingUrls.has(shown.preview.url)
      ) &&
      canCrossfadeLayers()
        ? shown
        : null;
    setLayers({ key: activeSourceKey, animate: contentShownRef.current, outgoing });
  }
  useLayoutEffect(() => {
    if (!activeSourceKey || !activeArtifact) return;
    shownLayerRef.current = {
      key: activeSourceKey,
      preview: visiblePreview,
      artifact: activeArtifact,
    };
    if (visiblePreview.kind !== 'loading') contentShownRef.current = true;
  });
  useEffect(() => {
    if (!layers.outgoing) return;
    const timer = window.setTimeout(
      () => setLayers((current) => (current.outgoing ? { ...current, outgoing: null } : current)),
      DUR.fast
    );
    return () => window.clearTimeout(timer);
  }, [layers.outgoing]);

  // Tell the host how tall this panel's content needs it to be. Measured on
  // CONTENT change only — the content's DOM changing (a file finishing loading, a
  // tab switch), its intrinsic box resizing (a re-wrap at a new width), a frame
  // reporting its document height, or the layout changing the panel's chrome —
  // never on the panel's own box resizing, which is the sheet the answer sizes.
  // One rAF coalesces a burst; nothing runs per frame at rest.
  const panelRef = useRef<HTMLElement | null>(null);
  const hasPanel = Boolean(activeArtifact && activeTab);
  const frameReportRef = useRef<{ source: MessageEvent['source']; height: number } | null>(null);
  useEffect(() => {
    const panel = panelRef.current;
    const body = previewBodyRef.current;
    if (!onContentHeightChange || !panel || !body) return;
    let frame = 0;
    let last: number | null | undefined;
    const observed = new Set<Element>();
    const resizeObserver =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => schedule());
    const observeIntrinsic = () => {
      const content = body.querySelector('[data-preview-layer="current"]') ?? body;
      const marked = content.querySelector('[data-preview-intrinsic]');
      const element =
        marked?.getAttribute('data-preview-intrinsic') === 'code'
          ? marked.querySelector('code')
          : marked;
      if (element && !observed.has(element)) {
        observed.add(element);
        resizeObserver?.observe(element);
      }
    };
    const measure = () => {
      frame = 0;
      observeIntrinsic();
      const next = measurePreviewContentHeight(panel, body, frameReportRef.current);
      if (next === undefined || next === last) return;
      last = next;
      onContentHeightChange(next);
    };
    function schedule() {
      if (!frame) frame = window.requestAnimationFrame(measure);
    }
    const mutationObserver =
      typeof MutationObserver === 'undefined' ? null : new MutationObserver(schedule);
    mutationObserver?.observe(body, { childList: true, subtree: true });
    const handleMessage = (event: MessageEvent) => {
      const data = event.data as { type?: unknown; height?: unknown } | undefined;
      if (!data || data.type !== PREVIEW_SIZE_MESSAGE_TYPE) return;
      const height = Number(data.height);
      if (!Number.isFinite(height) || height <= 0) return;
      // Only a frame inside THIS panel; a second panel's figure is not ours.
      const frames = body.querySelectorAll('iframe');
      if (![...frames].some((candidate) => candidate.contentWindow === event.source)) return;
      frameReportRef.current = { source: event.source, height };
      schedule();
    };
    window.addEventListener('message', handleMessage);
    schedule();
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
      window.removeEventListener('message', handleMessage);
    };
  }, [layout, onContentHeightChange, hasPanel]);

  if (!activeArtifact || !activeTab) return null;

  const activateTab = (tab: ArtifactTab) => {
    onUnfold?.();
    pendingNavigationKeyRef.current = artifactSourceKey(tab.artifact);
    dispatchTabAction({ type: 'activate', tabId: tab.id });
    onOpenArtifact(tab.artifact);
  };

  const beginTabPointerDrag = (event: PointerEvent<HTMLButtonElement>, tabId: string) => {
    if (event.button !== 0) return;
    tabPointerGestureRef.current = {
      tabId,
      startX: event.clientX,
      startY: event.clientY,
    };
  };

  const activateTabFromPointer = (tab: ArtifactTab) => {
    if (suppressTabClickRef.current) {
      suppressTabClickRef.current = false;
      window.clearTimeout(suppressTabClickTimerRef.current ?? undefined);
      suppressTabClickTimerRef.current = null;
      return;
    }
    activateTab(tab);
  };

  const openStandalone = async () => {
    if (activeArtifact.kind === 'html') {
      // Expand opens the offline preview in the user's default browser. Live
      // Agent Drafter apps use the explicit launch link returned by the tool.
      await window.electron.openArtifactInBrowser({
        html: visiblePreview.kind === 'html' ? visiblePreview.html : activeArtifact.html,
        title: activeArtifact.title,
        theme: resolvedTheme,
      });
      return;
    }
    if (activeArtifact.kind === 'externalUrl') {
      await window.electron.openExternal(activeArtifact.url);
      return;
    }
    if (activeArtifact.kind === 'file') {
      await window.electron.openDirectoryInExplorer(activeArtifact.path);
    }
  };

  const isBrowsingActive =
    activeArtifact.kind === 'externalUrl' && browsingUrls.has(activeArtifact.url);
  // A live page brings its own 32px toolbar (address, back, reload); it stands in
  // for the bar rather than stacking a second row above it.
  const livePage = isLivePagePreview(visiblePreview, isBrowsingActive);
  const revealLabel =
    window.electron?.platform === 'darwin'
      ? PREVIEW_COPY.revealInFinder
      : PREVIEW_COPY.showInFolder;
  const annotateUnavailable = annotateBrowserReason();
  const canQuote = Boolean(sessionId) && hasSelectedText(quotedSelection);
  const showMoreMenu = Boolean(sessionId) || activeArtifact.kind === 'file';

  const renderLayer = (
    key: string,
    role: 'current' | 'outgoing',
    layerPreview: PreviewState,
    layerArtifact: ArtifactSource,
    animate: boolean
  ) => (
    <div
      key={key}
      className="br-preview-layer"
      data-preview-layer={role}
      data-enter={role === 'current' && animate ? '' : undefined}
      aria-hidden={role === 'outgoing' ? true : undefined}
      // `inert` keeps a fading layer out of the tab order and the pointer's way.
      {...(role === 'outgoing' ? { inert: true } : {})}
    >
      <ArtifactPreviewBody
        preview={layerPreview}
        artifact={layerArtifact}
        resolvedTheme={resolvedTheme}
        isResizing={isResizing}
        trustedFrameRef={role === 'current' ? trustedFrameRef : outgoingFrameRef}
        onOpenArtifactInTab={openArtifactInTab}
        isBrowsingUrl={layerArtifact.kind === 'externalUrl' && browsingUrls.has(layerArtifact.url)}
        onStartBrowsing={(url) => setBrowsingUrls((current) => new Set(current).add(url))}
        onLiveBrowserViewChange={role === 'current' ? handleLiveBrowserViewChange : undefined}
        onLiveBrowserShareChange={role === 'current' ? onLiveBrowserShareChange : undefined}
        isAnnotating={role === 'current' && isAnnotating}
        annotationSnapshotDataUrl={
          role === 'current' ? (annotationSnapshot?.dataUrl ?? null) : null
        }
        refreshRevision={refreshRevision}
      />
    </div>
  );

  return (
    <>
      {onResizeStart && (
        // THE RESIZE EDGE IS THE PANEL'S SIBLING, NOT ITS CHILD. The panel clips
        // its own paint (`contain: paint`, `overflow: hidden`), so an edge inside
        // it could only ever sit over the preview's content. As a sibling, rung
        // 2's grid places it ON the seam (`.br-preview-resize-handle` in
        // main.css): the panel's left edge beside the conversation, and the
        // transcript's 8px top padding under a stacked sheet — covering nothing
        // either side can use. ONE element for both layouts, so a crossing
        // changes an attribute and mounts nothing.
        <div
          role="separator"
          aria-orientation={layout === 'stack' ? 'horizontal' : 'vertical'}
          aria-label={PREVIEW_COPY.resizePanel}
          onPointerDown={onResizeStart}
          className="br-preview-resize-handle"
        />
      )}
      <aside
        ref={panelRef}
        data-testid="artifact-viewer"
        // The anchor Ctrl+Tab arbitrates on: a keystroke landing inside this
        // subtree is aimed at the preview's tabs, anything else at the chat's.
        // Deliberately not the testid above — behaviour must not hang off a
        // promise we only made to tests.
        {...{ [ARTIFACT_PANEL_ATTR]: '' }}
        // The strip and the bar fade with the body (preview-panel.css), keyed on
        // the same open state the body's motion follows, so no part of the panel
        // appears or vanishes in a single frame.
        data-panel-open={isOpen ? 'true' : 'false'}
        style={{
          ...style,
          contain: 'layout paint',
        }}
        // Paper for every kind (spec 3.8): the strip keeps the band's ground and
        // everything under it is the page, so a switch never changes the ground.
        className={cn(
          'br-preview-panel no-drag relative isolate flex h-full min-h-0 w-full flex-col overflow-hidden',
          className
        )}
      >
        {/* The tab strip is `br-tabstrip` (shared, styles/main.css): its ground is the
          sidebar colour, so the window's whole 44px top edge is one continuous
          surface. `h-chrome` (44px): the third of the three bands that drop
          together, so this strip stays level with the chat header beside it. */}
        <div
          className="br-tabstrip no-drag relative z-50 h-chrome flex-shrink-0"
          // A folded sheet is only its strip, so the strip's bare ground is the
          // biggest target there is for "show me the preview again". Only the bare
          // ground: a tab, a menu or a button keeps its own meaning.
          onClick={(event) => {
            if (!folded) return;
            const target = event.target as HTMLElement;
            if (target === event.currentTarget || target === tabListRef.current) onUnfold?.();
          }}
        >
          {/* The tablist nests inside the strip for a11y, so it repeats the strip's
            own 3px gap: `.br-tab + .br-tab::before` hangs its divider at -2px and
            only lands in the gap if the tabs are spaced the way the class expects. */}
          <div
            ref={tabListRef}
            role="tablist"
            aria-label={PREVIEW_COPY.tabList}
            aria-keyshortcuts="Meta+W Control+W Control+Tab Control+Shift+Tab"
            // Rung 3 of the yield ladder (D-32): shrink to the floor, then SCROLL,
            // then collapse into a ▾ — never wrap. The edge that has tabs scrolled
            // past it fades (preview-panel.css), so a cut-off tab reads as more.
            data-scrolled-start={tabListScrolled.start ? '' : undefined}
            data-scrolled-end={tabListScrolled.end ? '' : undefined}
            className="br-tabstrip__scroll flex min-w-0 flex-1 items-center gap-[3px] overflow-x-auto"
          >
            {tabState.tabs.map((tab) => {
              const TabIcon = iconForArtifact(tab.artifact);
              const isActive = tab.id === tabState.activeTabId;
              return (
                // `br-tab` (shared) paints the tab, and its drag rules do the
                // feedback: `data-dragging` dims the tab being carried and
                // `data-dropbefore` draws the insertion hairline on the tab it will
                // land on — the same two attributes ChatTabStrip writes.
                <div
                  key={tab.id}
                  data-artifact-tab-id={tab.id}
                  data-active={isActive ? 'true' : undefined}
                  data-dragging={draggedTabId === tab.id ? 'true' : undefined}
                  data-dropbefore={dragOverTabId === tab.id ? 'true' : undefined}
                  className="br-tab group"
                >
                  <button
                    ref={isActive ? activeTabButtonRef : undefined}
                    type="button"
                    role="tab"
                    aria-selected={isActive}
                    aria-controls={previewContentId}
                    onPointerDown={(event) => beginTabPointerDrag(event, tab.id)}
                    onClick={() => activateTabFromPointer(tab)}
                    title={artifactHoverTitle(tab.artifact)}
                    className="flex h-full min-w-0 flex-1 cursor-grab items-center gap-1.5 text-left active:cursor-grabbing"
                  >
                    <TabIcon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                    <span className="br-tab__label min-w-0 flex-1 truncate">
                      {tab.artifact.title}
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={() => closeTab(tab.id)}
                    aria-label={PREVIEW_COPY.closeTab(tab.artifact.title)}
                    title={PREVIEW_COPY.closeTab(tab.artifact.title)}
                    className={cn(
                      // A 14px glyph in a 20px hit area: the close yields to the
                      // label rather than matching the kind icon's weight.
                      'inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-inner text-text-subtle transition-[background-color,color,opacity] hover:bg-overlay-hover hover:text-text-default',
                      isActive
                        ? 'opacity-100'
                        : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'
                    )}
                  >
                    <X className="h-3.5 w-3.5" aria-hidden="true" />
                  </button>
                </div>
              );
            })}
          </div>
          {showTabOverflowMenu && (
            // Outside the scroll box, for the reason ChatTabStrip's wrap documents:
            // inside it, the button's own width would keep alive the overflow that
            // summoned it.
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label={PREVIEW_COPY.overflow}
                  data-testid="artifact-tab-overflow-trigger"
                  className="br-tabstrip__overflow"
                >
                  <ChevronDown className="h-4 w-4" aria-hidden="true" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="max-h-[60vh] w-56 overflow-y-auto">
                {tabState.tabs.map((tab) => {
                  const TabIcon = iconForArtifact(tab.artifact);
                  return (
                    <DropdownMenuItem
                      key={tab.id}
                      data-testid={`artifact-tab-overflow-item-${tab.id}`}
                      onSelect={() => activateTab(tab)}
                      className={cn('gap-2', tab.id === tabState.activeTabId && 'font-medium')}
                    >
                      <TabIcon className="h-4 w-4 flex-none" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate">{tab.artifact.title}</span>
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          {/* Two actions always, the rest when wanted (spec 3.8): Open externally
              and Close; Quote while text is selected; the pressed region tool
              while a region is being picked; the refresh dot while an update
              waits; everything else in ⋯. */}
          <div className="br-preview-actions">
            {canQuote && (
              <PreviewIconButton
                label={PREVIEW_COPY.quoteSelection}
                data-testid="artifact-quote"
                // Keep the selection: a press on a button would collapse it first.
                onMouseDown={(event) => event.preventDefault()}
                onClick={() =>
                  attachSelectedText(
                    quoteSource,
                    quotedSelection,
                    previewBodyRef.current ?? undefined
                  )
                }
              >
                <MessageSquareText className="h-4 w-4" aria-hidden="true" />
              </PreviewIconButton>
            )}
            {isAnnotating && (
              <PreviewIconButton
                label={PREVIEW_COPY.cancelAnnotate}
                data-testid="artifact-annotate-cancel"
                aria-pressed={true}
                onClick={() => void toggleAnnotation()}
                className="br-preview-icon-button--pressed"
              >
                <Camera className="h-4 w-4" aria-hidden="true" />
              </PreviewIconButton>
            )}
            {(updateReady || refreshFailed) && activeArtifact.kind === 'file' && (
              <PreviewIconButton
                label={refreshFailed ? PREVIEW_COPY.retryUpdate : PREVIEW_COPY.updateReady}
                hint={refreshFailed ? PREVIEW_COPY.retryUpdateHint : PREVIEW_COPY.updateReadyHint}
                data-testid="artifact-update"
                disabled={isAnnotating || isResizing}
                onClick={() => {
                  refreshTrackerRef.current.revision = refreshRevision;
                  deferredRefreshRef.current = false;
                  manualRefreshRef.current = true;
                  setUpdateReady(false);
                  setFileRefresh((previous) => ({
                    scope: refreshScope,
                    revision: previous.revision + 1,
                  }));
                }}
                className="relative"
              >
                <RefreshCw className="h-4 w-4" aria-hidden="true" />
                <span className="br-preview-update-dot" aria-hidden="true" />
              </PreviewIconButton>
            )}
            {activeArtifact.kind !== 'mcpResource' && (
              <PreviewIconButton label={PREVIEW_COPY.openExternally} onClick={openStandalone}>
                <ExternalLink className="h-4 w-4" aria-hidden="true" />
              </PreviewIconButton>
            )}
            {showMoreMenu && (
              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <button
                        type="button"
                        aria-label={PREVIEW_COPY.moreActions}
                        data-testid="artifact-more-actions"
                        className={cn(
                          buttonVariants({ variant: 'ghost', shape: 'round' }),
                          'br-preview-icon-button no-drag'
                        )}
                      >
                        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
                      </button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent side="bottom">{PREVIEW_COPY.moreActions}</TooltipContent>
                </Tooltip>
                <DropdownMenuContent align="end" className="min-w-[180px] max-w-[320px]">
                  {sessionId && (
                    // Annotation is available on EVERY preview kind, not just one:
                    // a researcher reads a generated report and needs to point at
                    // something in it. For this audience the report IS the artifact.
                    <DropdownMenuItem
                      data-testid="artifact-annotate"
                      disabled={annotateUnavailable !== null}
                      onSelect={() => void toggleAnnotation()}
                      // The name stays the action; the reason it is off is the
                      // item's description, and it stays visible (principle 2:
                      // a refusal says why before the item is ever pressed).
                      aria-label={PREVIEW_COPY.annotate}
                      aria-describedby={annotateUnavailable ? annotateReasonId : undefined}
                      className="flex-col items-start gap-0.5"
                    >
                      <span>{PREVIEW_COPY.annotate}</span>
                      {annotateUnavailable && (
                        <span id={annotateReasonId} className="text-supporting text-text-muted">
                          {annotateUnavailable}
                        </span>
                      )}
                    </DropdownMenuItem>
                  )}
                  {activeArtifact.kind === 'file' && (
                    <DropdownMenuItem
                      data-testid="artifact-reveal"
                      onSelect={() =>
                        void window.electron.openDirectoryInExplorer(
                          dirnameFromPath(activeArtifact.path)
                        )
                      }
                    >
                      {revealLabel}
                    </DropdownMenuItem>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
            {onToggleFold && (
              // Rendered in both layouts and hidden beside the conversation by
              // main.css (`.br-preview-fold-toggle`): a side column has nothing to
              // fold, and a crossing must not mount or unmount a control.
              <PreviewIconButton
                data-testid="artifact-fold-toggle"
                onClick={onToggleFold}
                aria-expanded={!folded}
                aria-controls={previewContentId}
                className="br-preview-fold-toggle"
                label={folded ? PREVIEW_COPY.unfoldPreview : PREVIEW_COPY.foldPreview}
              >
                {/* One glyph, turned by main.css when folded: swapping two icons
                    would mount and unmount an element on every fold. */}
                <ChevronUp className="h-4 w-4" aria-hidden="true" />
              </PreviewIconButton>
            )}
            <PreviewIconButton label={PREVIEW_COPY.closePanel} onClick={onClose}>
              <X className="h-4 w-4" aria-hidden="true" />
            </PreviewIconButton>
          </div>
        </div>

        <PreviewBarScope>
          {/* The one bar (spec 3.8), from the artifact's own name before its
              content loads, so a switch never takes it away. `br-preview-measure-
              strip` puts its first glyph on the reading column's left edge. */}
          {!livePage && (
            <PreviewBar
              name={activeArtifact.title}
              path={artifactHoverTitle(activeArtifact)}
              className="br-preview-measure-strip"
            />
          )}
          <QuotedTextSelection source={quoteSource} selection={sessionId ? quotedSelection : ''}>
            <div
              id={previewContentId}
              data-testid="artifact-preview-content"
              data-preview-open={isOpen ? 'true' : 'false'}
              ref={previewBodyRef}
              className="br-preview-body relative z-0 min-h-0 flex-1 overflow-hidden"
            >
              {isResizing && (
                <div
                  data-testid="artifact-resize-shield"
                  aria-hidden="true"
                  className="absolute inset-0 z-50 cursor-col-resize"
                />
              )}
              {isAnnotating && (
                <AnnotationOverlay
                  onCancel={finishAnnotation}
                  onSelect={(region) => {
                    void captureAnnotation(region);
                  }}
                />
              )}
              {layers.outgoing &&
                renderLayer(
                  layers.outgoing.key,
                  'outgoing',
                  layers.outgoing.preview,
                  layers.outgoing.artifact,
                  false
                )}
              {renderLayer(
                activeSourceKey ?? activeTab.id,
                'current',
                visiblePreview,
                activeArtifact,
                layers.animate
              )}
            </div>
          </QuotedTextSelection>
        </PreviewBarScope>
      </aside>
    </>
  );
}

/**
 * Friendly empty-state for a file that could not be read (#36). The main
 * process already maps errno codes to human-readable messages (see
 * utils/artifactFileErrors.ts); this renders them as a proper centered state
 * — icon, short title, message, muted path — instead of a bare gray line of
 * error text.
 */
function ArtifactErrorState({
  message,
  path,
  code,
}: {
  message: string;
  path?: string;
  code?: string;
}) {
  const Icon = code === 'EACCES' || code === 'EPERM' ? Lock : FileX;
  // An error stays visible (principle 2): the icon, the title and the daemon's
  // own sentence. The path is a machine string, so it is the title's tooltip.
  return (
    <div data-testid="artifact-error-state" className="br-preview-state">
      <Icon className="h-5 w-5 text-text-muted" aria-hidden="true" />
      <div className="text-label text-text-default" title={path}>
        {PREVIEW_COPY.fileNotAvailable}
      </div>
      <p className="br-preview-state__line text-body text-text-muted">{message}</p>
    </div>
  );
}

function ArtifactPreviewBody({
  preview,
  artifact,
  resolvedTheme,
  isResizing,
  trustedFrameRef,
  onOpenArtifactInTab,
  isBrowsingUrl,
  onStartBrowsing,
  onLiveBrowserViewChange = () => {},
  onLiveBrowserShareChange,
  isAnnotating = false,
  annotationSnapshotDataUrl = null,
  refreshRevision = 0,
}: {
  preview: PreviewState;
  artifact: ArtifactSource;
  resolvedTheme: 'light' | 'dark';
  isResizing: boolean;
  trustedFrameRef: React.RefObject<HTMLIFrameElement | null>;
  onOpenArtifactInTab: (artifact: ArtifactSource) => void;
  /** The user has clicked "Open here" for this URL in this tab. */
  isBrowsingUrl: boolean;
  onStartBrowsing?: (url: string) => void;
  onLiveBrowserViewChange?: (viewId: string | null) => void;
  onLiveBrowserShareChange?: (share: LiveBrowserShare | null) => void;
  isAnnotating?: boolean;
  annotationSnapshotDataUrl?: string | null;
  refreshRevision?: number;
}) {
  if (preview.kind === 'loading') {
    // A first visit only (a tab that has been seen shows its last preview). It
    // shows nothing for 200ms — most reads land sooner — then three quiet lines
    // on the paper (preview-panel.css). No word: the bar above already names
    // what is coming.
    return <PreviewSkeleton />;
  }

  if (preview.kind === 'error') {
    return <ArtifactErrorState message={preview.message} />;
  }

  if (preview.kind === 'html') {
    // `allow-popups` is withheld: with it, figure HTML can window.open() a
    // `data:` URL, which the main window's open handler turns into a real
    // BrowserWindow that inherits the preload IPC bridge.
    return (
      <iframe
        name="biorouter-artifact-preview"
        key={preview.html}
        ref={trustedFrameRef}
        aria-label={artifact.title}
        // Inject the app theme so this preview matches the expanded/opened view,
        // which loads with an explicit `?theme=`; a srcdoc iframe has no query.
        srcDoc={injectArtifactBrowserCsp(
          withPreviewSizeReporting(withHostTheme(preview.html, resolvedTheme))
        )}
        sandbox="allow-scripts allow-downloads"
        // Held transparent until its document has painted, then faded in
        // (preview-panel.css): the white page ground stays, but it never
        // flashes over a dark panel before the figure is drawn.
        data-loaded="false"
        onLoad={markFrameLoaded}
        className={cn(
          'br-preview-frame h-full w-full bg-white',
          isResizing && 'pointer-events-none'
        )}
      />
    );
  }

  if (preview.kind === 'externalUrl') {
    // Ordinary live pages open **only** on a deliberate click. The sole
    // automatic case is a main-process-approved app served by this window's
    // active BioRouter daemon; WebPagePreview creation independently rechecks
    // that same scope before granting its managed-app request policy.
    //
    // This card is the whole boundary between "the user browsed somewhere" and
    // "something else navigated the user's app". An MCP resource link with an
    // http(s) URI already becomes an artifact with no transcript card and can
    // auto-open; if that path rendered a live view directly, any extension
    // could make an arbitrary site load and execute here. One click is cheap
    // for the user and structurally impossible for the agent.
    if (preview.managedApp || isBrowsingUrl) {
      return (
        <WebPagePreview
          key={preview.url}
          url={preview.url}
          requireManagedApp={preview.managedApp}
          refreshRevision={refreshRevision}
          // The native view has no shared z-index with the DOM, so it has to be
          // hidden while the resize shield is up or it paints straight over it.
          isSuspended={isResizing || isAnnotating}
          snapshotDataUrl={annotationSnapshotDataUrl}
          onViewIdChange={onLiveBrowserViewChange}
          onAgentShareChange={onLiveBrowserShareChange}
          onOpenExternal={(url) => void window.electron.openExternal(url)}
        />
      );
    }

    // On the paper, no card and no shadow (spec 3.8): the URL and two choices.
    return (
      <div data-testid="artifact-external-page" className="br-preview-state">
        <Globe className="h-5 w-5 text-text-muted" aria-hidden="true" />
        <div className="br-preview-state__line break-all text-body text-text-default">
          {preview.url}
        </div>
        <div className="mt-2 flex flex-wrap items-center justify-center gap-2">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            data-testid="artifact-open-here"
            onClick={() => onStartBrowsing?.(preview.url)}
          >
            <Globe className="h-4 w-4" aria-hidden="true" />
            {PREVIEW_COPY.openHere}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void window.electron.openExternal(preview.url)}
          >
            <ExternalLink className="h-4 w-4" aria-hidden="true" />
            {PREVIEW_COPY.openInBrowser}
          </Button>
        </div>
      </div>
    );
  }

  if (preview.kind === 'mcpResource' && artifact.kind === 'mcpResource') {
    return (
      <UIResourceRenderer
        resource={artifact.resource}
        supportedContentTypes={['rawHtml']}
        htmlProps={{
          autoResizeIframe: { height: false, width: false },
          style: { width: '100%', height: '100%', minHeight: '100%', border: 'none' },
          iframeRenderData: { host: 'biorouter', theme: resolvedTheme },
        }}
      />
    );
  }

  if (preview.kind !== 'file') return null;
  const file = preview.preview;

  if (file.kind === 'error') {
    return (
      <ArtifactErrorState
        // A path the assistant only NAMED never existed, so the generic ENOENT
        // copy ("moved, renamed, or deleted") would assert a history it does not
        // have. `mentionedOnly` survives only when nothing confirmed the path.
        message={artifactFileErrorMessage(file, {
          mentionedOnly: artifact.kind === 'file' && artifact.mentionedOnly === true,
        })}
        path={file.path}
        code={file.code}
      />
    );
  }

  if (file.kind === 'image') {
    return <ImageFilePreview key={file.path} file={file} />;
  }

  if (file.kind === 'document') {
    return <DocumentPreview file={file} resolvedTheme={resolvedTheme} isResizing={isResizing} />;
  }

  if ((file.kind === 'text' || file.kind === 'html') && extensionFromPath(file.path) === 'ipynb') {
    return <NotebookPreview file={file} resolvedTheme={resolvedTheme} />;
  }

  if (file.kind === 'gitDirectory' || file.kind === 'directory') {
    return (
      <DirectoryTreePreview
        key={file.path}
        directory={file}
        resolvedTheme={resolvedTheme}
        isResizing={isResizing}
        onOpenArtifactInTab={onOpenArtifactInTab}
      />
    );
  }

  if (file.kind === 'binary') {
    // Text-decodable files already fall through to the plain-text preview below
    // (see isTextArtifact in the main process). Reaching here means the bytes are
    // genuinely binary (or the file is too large), so there is nothing to show.
    //
    // Where we know *which* format this is and why we decline it, say so. One
    // generic sentence for a .doc, a .heic and a .key leaves the user unable to
    // tell whether their file is broken, the app is broken, or the format was
    // never supported.
    const unsupported = describeUnsupportedFormat(extensionFromPath(file.path));
    const name = basenameFromPath(file.path);
    // The name, `DOC · 24 KB` and one button (spec 3.8). Why it is not shown,
    // and what to do instead, is the InfoTip beside the format: one hover, one
    // Tab, and read aloud with the format line.
    const reason = [
      unsupported ? unsupported.reason : PREVIEW_COPY.binaryFallbackReason,
      unsupported?.suggestion,
    ]
      .filter(Boolean)
      .join(' ');
    return (
      <div data-testid="artifact-binary-file" className="br-preview-state">
        <File className="h-5 w-5 text-text-muted" aria-hidden="true" />
        <div className="text-label text-text-default">{name}</div>
        <div className="flex items-center gap-1 text-supporting text-text-muted">
          <span className="tabular-nums">
            {unsupported ? `${unsupported.label} · ` : `${file.mimeType} · `}
            {formatBytes(file.size)}
          </span>
          <InfoTip label={name} help={reason} />
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="mt-2"
          onClick={() => window.electron.openDirectoryInExplorer(file.path)}
        >
          <ExternalLink className="h-4 w-4" aria-hidden="true" />
          {PREVIEW_COPY.openFile}
        </Button>
      </div>
    );
  }

  // Keyed by path so the Preview/Raw toggle resets when a different file opens —
  // the panel stays mounted across artifacts, so the state would otherwise stick
  // and show a CSV as raw text just because the last markdown was.
  if (file.kind === 'text' || file.kind === 'html') {
    return (
      <TextFilePreview
        key={file.path}
        file={file}
        resolvedTheme={resolvedTheme}
        onOpenArtifact={onOpenArtifactInTab}
        sourceLine={artifact.kind === 'file' ? artifact.line : undefined}
      />
    );
  }
  return null;
}

const gitStatusPresentation: Record<ArtifactGitStatus, { label: string; dotClass: string }> = {
  untracked: { label: 'Untracked', dotClass: 'bg-background-info' },
  modified: { label: 'Modified', dotClass: 'bg-background-warning' },
  staged: { label: 'Staged', dotClass: 'bg-background-success' },
  // ⚠ NOT the danger hue. A committed file is the settled, wanted state, and
  // the app's error colour said something had gone wrong with the file the user
  // had just finished with. Muted ink puts it where "nothing to do here"
  // belongs, beside `pushed`.
  committed: { label: 'Committed', dotClass: 'bg-text-muted' },
  pushed: { label: 'Pushed', dotClass: 'bg-text-muted' },
};

type ArtifactTreeDirectory = Extract<ArtifactFilePreview, { kind: 'directory' | 'gitDirectory' }>;
type ArtifactTreeEntry = ArtifactFileEntry | ArtifactGitEntry;

function DirectoryTreePreview({
  directory,
  resolvedTheme,
  isResizing,
  onOpenArtifactInTab,
}: {
  directory: ArtifactTreeDirectory;
  resolvedTheme: 'light' | 'dark';
  isResizing: boolean;
  onOpenArtifactInTab: (artifact: ArtifactSource) => void;
}) {
  const isGitRepository = directory.kind === 'gitDirectory';
  const treeEntries = useMemo<ArtifactTreeEntry[]>(
    () =>
      directory.entries.map((entry) => {
        const incoming = entry as ArtifactTreeEntry & {
          relativePath?: string;
          parentPath?: string;
        };
        return {
          ...entry,
          relativePath: incoming.relativePath ?? entry.name,
          parentPath: incoming.parentPath ?? '',
        } as ArtifactTreeEntry;
      }),
    [directory.entries]
  );
  const [filter, setFilter] = useState('');
  const [expandedDirectories, setExpandedDirectories] = useState<Set<string>>(
    () =>
      new Set(
        treeEntries
          .filter((entry) => entry.isDirectory && entry.relativePath.split('/').length <= 2)
          .map((entry) => entry.relativePath)
      )
  );
  const [selectedEntry, setSelectedEntry] = useState<ArtifactTreeEntry | null>(null);
  const [selectedPreview, setSelectedPreview] = useState<ArtifactFilePreview | null>(null);
  const selectedRequestRef = useRef(0);
  const selectedFrameRef = useRef<HTMLIFrameElement | null>(null);

  const childEntries = useMemo(() => {
    const children = new Map<string, ArtifactTreeEntry[]>();
    for (const entry of treeEntries) {
      const siblings = children.get(entry.parentPath) ?? [];
      siblings.push(entry);
      children.set(entry.parentPath, siblings);
    }
    return children;
  }, [treeEntries]);

  const visibleEntries = useMemo(() => {
    const query = filter.trim().toLocaleLowerCase();
    const includedPaths = new Set<string>();
    if (query) {
      for (const entry of treeEntries) {
        if (!entry.relativePath.toLocaleLowerCase().includes(query)) continue;
        includedPaths.add(entry.relativePath);
        const segments = entry.relativePath.split('/');
        for (let index = 1; index < segments.length; index += 1) {
          includedPaths.add(segments.slice(0, index).join('/'));
        }
      }
    }

    const result: ArtifactTreeEntry[] = [];
    const appendChildren = (parentPath: string) => {
      for (const entry of childEntries.get(parentPath) ?? []) {
        if (query && !includedPaths.has(entry.relativePath)) continue;
        result.push(entry);
        if (entry.isDirectory && (query || expandedDirectories.has(entry.relativePath))) {
          appendChildren(entry.relativePath);
        }
      }
    };
    appendChildren('');
    return result;
  }, [childEntries, expandedDirectories, filter, treeEntries]);

  const openTreeFile = useCallback(async (entry: ArtifactTreeEntry) => {
    const request = selectedRequestRef.current + 1;
    selectedRequestRef.current = request;
    setSelectedEntry(entry);
    setSelectedPreview(null);
    try {
      let response: ArtifactFilePreview = await window.electron.readArtifactFile(entry.path);
      if (response.kind === 'html') {
        try {
          const prepared = await window.electron.prepareArtifactHtml({
            html: response.text,
          });
          response = { ...response, preparedHtml: prepared.html };
        } catch {
          response = { ...response, preparedHtml: response.text };
        }
      }
      if (selectedRequestRef.current === request) setSelectedPreview(response);
    } catch (cause) {
      if (selectedRequestRef.current !== request) return;
      setSelectedPreview({
        kind: 'error',
        title: entry.name,
        path: entry.path,
        error: cause instanceof Error ? cause.message : 'Could not open this folder file.',
        found: false,
      });
    }
  }, []);

  return (
    // The rail is real structure, so it keeps its hairline — but not a second
    // ground: both halves sit on the panel's, divided by the one border. The
    // directory's name is the bar's; its branch is the bar's meta.
    <div className="flex h-full min-h-0" data-preview-opaque="">
      {directory.kind === 'gitDirectory' && (
        <PreviewBarMeta>
          <span
            data-testid="artifact-directory-branch"
            className="br-preview-bar__ident"
            title={directory.branch}
          >
            {directory.branch}
          </span>
        </PreviewBarMeta>
      )}
      <div className="br-preview-rail">
        {/* The rail's header is the filter, one 32px row (the bar's height), so
            the two hairlines either side of the seam line up. */}
        <label className="br-preview-rail__filter">
          <Search className="h-3.5 w-3.5 shrink-0 text-text-subtle" aria-hidden="true" />
          <input
            type="search"
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder={PREVIEW_COPY.filterFiles}
            aria-label={isGitRepository ? PREVIEW_COPY.filterRepository : PREVIEW_COPY.filterFolder}
            className="min-w-0 flex-1 bg-transparent text-secondary text-text-default outline-none placeholder:text-text-subtle"
          />
        </label>
        <div
          role="tree"
          aria-label={`${directory.title} ${isGitRepository ? 'repository' : 'folder'} files`}
          className="min-h-0 flex-1 overflow-auto p-1.5"
        >
          {visibleEntries.map((entry) => {
            const depth = entry.relativePath.split('/').length - 1;
            const status = isGitRepository
              ? gitStatusPresentation[(entry as ArtifactGitEntry).status]
              : null;
            const selected = selectedEntry?.relativePath === entry.relativePath;
            const expanded = filter.trim() !== '' || expandedDirectories.has(entry.relativePath);
            // Git status is said ONCE on the row: a coloured dot, with the word in
            // the row's name and tooltip (no coloured name, no legend).
            const name = [
              entry.name,
              status?.label,
              selected ? PREVIEW_COPY.currentlyViewing : null,
            ]
              .filter(Boolean)
              .join(', ');
            return (
              <button
                type="button"
                role="treeitem"
                aria-expanded={entry.isDirectory ? expanded : undefined}
                aria-selected={selected}
                aria-current={selected ? 'true' : undefined}
                aria-label={name}
                key={entry.relativePath}
                title={status ? `${entry.relativePath} · ${status.label}` : entry.relativePath}
                onClick={(event) => {
                  if (event.detail > 1) return;
                  if (!entry.isDirectory) {
                    void openTreeFile(entry);
                    return;
                  }
                  setExpandedDirectories((current) => {
                    const next = new Set(current);
                    if (next.has(entry.relativePath)) next.delete(entry.relativePath);
                    else next.add(entry.relativePath);
                    return next;
                  });
                }}
                onDoubleClick={() => {
                  if (entry.isDirectory) return;
                  onOpenArtifactInTab({
                    kind: 'file',
                    title: entry.name,
                    path: entry.path,
                  });
                }}
                className={cn(
                  // A row in a list, not a card: selection is a fill, never a lift.
                  'group flex h-7 w-full min-w-0 items-center gap-1.5 rounded-element px-1.5 text-left text-secondary text-text-default transition-colors',
                  selected ? 'bg-overlay-selected' : 'hover:bg-overlay-hover'
                )}
                style={{ paddingLeft: `${6 + depth * 12}px` }}
              >
                {entry.isDirectory ? (
                  expanded ? (
                    <ChevronDown
                      className="h-3.5 w-3.5 shrink-0 text-text-subtle"
                      aria-hidden="true"
                    />
                  ) : (
                    <ChevronRight
                      className="h-3.5 w-3.5 shrink-0 text-text-subtle"
                      aria-hidden="true"
                    />
                  )
                ) : (
                  <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center">
                    {status && <span className={cn('h-1.5 w-1.5 rounded-full', status.dotClass)} />}
                  </span>
                )}
                {entry.isDirectory ? (
                  <Folder className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden="true" />
                ) : (
                  <FileText className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden="true" />
                )}
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              </button>
            );
          })}
          {visibleEntries.length === 0 && (
            <div className="px-2 py-6 text-center text-supporting text-text-muted">
              {filter.trim() ? PREVIEW_COPY.noMatchingFiles : PREVIEW_COPY.emptyFolder}
            </div>
          )}
        </div>
      </div>
      <div className="min-w-0 flex-1">
        {selectedEntry ? (
          <ArtifactPreviewBody
            preview={selectedPreview ? { kind: 'file', preview: selectedPreview } : LOADING_PREVIEW}
            artifact={{ kind: 'file', title: selectedEntry.name, path: selectedEntry.path }}
            resolvedTheme={resolvedTheme}
            isResizing={isResizing}
            trustedFrameRef={selectedFrameRef}
            onOpenArtifactInTab={onOpenArtifactInTab}
            // A directory tree only ever selects files, never URLs, so there is
            // no live page to be browsing here.
            isBrowsingUrl={false}
          />
        ) : (
          // One line, no paragraph (spec 3.8): the tree beside it explains itself.
          <div className="br-preview-state">
            {isGitRepository ? (
              <Github className="h-5 w-5 text-text-subtle" aria-hidden="true" />
            ) : (
              <Folder className="h-5 w-5 text-text-subtle" aria-hidden="true" />
            )}
            <div className="text-body text-text-muted">{PREVIEW_COPY.selectFile}</div>
          </div>
        )}
      </div>
    </div>
  );
}

function ImageFilePreview({
  file: preview,
}: {
  file: Extract<ArtifactFilePreview, { kind: 'image' }>;
}) {
  // The src is derived in an effect, not inline, because a large image arrives
  // as bytes and becomes a `blob:` URL that has to be revoked. Building it
  // during render would mint a fresh URL on every re-render and leak all but
  // the last.
  const [src, setSrc] = useState('');
  const [failed, setFailed] = useState(false);
  const [fit, setFit] = useState<'fit' | 'actual'>('fit');
  const [natural, setNatural] = useState<{ width: number; height: number } | null>(null);

  useEffect(() => {
    setFailed(false);
    let revoke: (() => void) | null = null;
    let cancelled = false;

    // TIFF has no decoder in any browser, so it is decoded here before display.
    // Done in the renderer, lazily, exactly like the four document renderers
    // beside it — the alternative was a daemon round-trip, which would give
    // image preview a failure mode (daemon down, no picture) that it does not
    // have today.
    if (preview.mimeType === 'image/tiff' && preview.bytes) {
      const bytes = preview.bytes;
      void import('utif2')
        .then(async (UTIF) => {
          if (cancelled) return;
          const pages = UTIF.decode(bytes);
          if (!pages.length) throw new Error('no pages');
          // The first page only. Multi-page TIFF is real in microscopy and
          // deserves a page control; showing page 1 is the honest first step,
          // not a claim to have handled the stack.
          const { width, height, rgbaBytes } = safeTiffDimensions(pages[0]);
          UTIF.decodeImage(bytes, pages[0]);
          const rgba = UTIF.toRGBA8(pages[0]);
          if (rgba.byteLength !== rgbaBytes) throw new Error('invalid decoded pixel data');
          const canvas = document.createElement('canvas');
          canvas.width = width;
          canvas.height = height;
          const ctx = canvas.getContext('2d');
          if (!ctx) throw new Error('no 2d context');
          ctx.putImageData(
            new ImageData(new Uint8ClampedArray(rgba), canvas.width, canvas.height),
            0,
            0
          );
          const blob = await new Promise<Blob | null>((resolve) =>
            canvas.toBlob(resolve, 'image/png')
          );
          canvas.width = 0;
          canvas.height = 0;
          if (!blob || cancelled) return;
          const url = URL.createObjectURL(blob);
          revoke = () => URL.revokeObjectURL(url);
          setSrc(url);
        })
        .catch(() => {
          if (!cancelled) setFailed(true);
        });
      return () => {
        cancelled = true;
        revoke?.();
        setSrc('');
      };
    }

    const source = imageSourceForPreview(preview);
    revoke = source.revoke;
    setSrc(source.src);
    return () => {
      cancelled = true;
      revoke?.();
      setSrc('');
    };
  }, [preview]);

  if (failed) {
    return (
      <ArtifactErrorState
        // By the time this renders the format was one we claim to handle —
        // natively, or through the TIFF decoder, or through the OS. So the
        // honest reading is a bad file, not a missing feature.
        message={PREVIEW_COPY.imageUndecodable}
        path={preview.path}
      />
    );
  }

  const meta = [
    natural ? `${natural.width}×${natural.height}` : null,
    preview.size !== undefined ? formatBytes(preview.size) : null,
  ]
    .filter(Boolean)
    .join(' · ');

  // The image is the content, so it sits on the paper with a gutter — no tinted
  // well behind it, no radius pretending it is a card. Fit shows it whole; 1:1
  // shows its real pixels and scrolls. It fades in once decoded, never pops.
  return (
    <>
      <PreviewBarMeta>
        {meta && (
          <span data-testid="artifact-strip-count" className="tabular-nums">
            {meta}
          </span>
        )}
      </PreviewBarMeta>
      <PreviewBarActions>
        <ViewSwitch
          value={fit}
          options={[
            { value: 'fit' as const, label: PREVIEW_COPY.fit },
            { value: 'actual' as const, label: PREVIEW_COPY.actualSize },
          ]}
          onChange={setFit}
        />
      </PreviewBarActions>
      <div className="br-preview-image" data-fit={fit}>
        {src ? (
          <img
            src={src}
            alt={preview.title}
            data-loaded="false"
            onLoad={(event) => {
              const image = event.currentTarget;
              image.dataset.loaded = 'true';
              if (image.naturalWidth > 0) {
                setNatural({ width: image.naturalWidth, height: image.naturalHeight });
              }
            }}
            onError={() => setFailed(true)}
          />
        ) : null}
      </div>
    </>
  );
}

function CodeBlock({
  text,
  language,
  resolvedTheme,
  sourceLine,
}: {
  text: string;
  language: string;
  resolvedTheme: 'light' | 'dark';
  sourceLine?: number;
}) {
  const lineCount = countLines(text);
  const theme = codeThemesByFamily[useThemeFamily()][resolvedTheme];
  // The gutter is quiet by fading its INK, not the element: an `opacity` would
  // fade the sticky gutter's opaque paper ground too, and a long line scrolled
  // under it showed through the numbers. How far it fades is GUTTER_INK_MIX,
  // held to 3:1 on the paper in every family by codeTheme.test.ts.
  const codeStyle = useMemo(() => withFadedGutter(theme, GUTTER_INK_MIX), [theme]);
  const codeRef = useRef<HTMLDivElement>(null);
  const numbered = lineCount > 1 && lineCount <= MAX_LINE_NUMBERED_LINES;
  const selectedLine =
    typeof sourceLine === 'number' &&
    Number.isSafeInteger(sourceLine) &&
    sourceLine > 0 &&
    sourceLine <= lineCount &&
    lineCount <= MAX_LINE_NUMBERED_LINES
      ? sourceLine
      : undefined;
  useEffect(() => {
    if (selectedLine) {
      codeRef.current
        ?.querySelector(`[data-source-line="${selectedLine}"]`)
        ?.scrollIntoView?.({ block: 'center' });
    }
  }, [selectedLine, text]);
  return (
    <div
      ref={codeRef}
      className="br-paper-code min-h-full"
      data-numbered={numbered || undefined}
      data-preview-intrinsic="code"
    >
      <SyntaxHighlighter
        style={codeStyle}
        language={language}
        PreTag="div"
        showLineNumbers={numbered}
        // Every numbered line is its own element (`[data-source-line]`), so the
        // gutter can stick while long lines scroll under it and a requested source
        // line paints its whole row. Still no `wrapLongLines`: combined with
        // `showLineNumbers` the highlighter makes every line `display: flex`
        // (highlight.js:106), which turns each token into a flex item and shreds
        // the line across the panel's width. Long lines scroll horizontally
        // instead, which is what a code viewer should do anyway — and it keeps
        // indentation honest.
        wrapLines={numbered || selectedLine !== undefined}
        lineProps={(lineNumber) => ({
          'data-source-line': lineNumber,
          ...(lineNumber === selectedLine
            ? {
                'aria-current': 'location' as const,
                style: { background: 'var(--background-medium)' },
              }
            : {}),
        })}
        lineNumberStyle={{
          // The gutter is the lead plus a FIXED-width number (not the library's
          // digits-based width); the margin before it is the line's own padding
          // (main.css, `.br-paper-code[data-numbered] [data-source-line]`). The
          // three add up to the paper column's edge, so code text lands exactly
          // there and aligns with a report's prose at every panel width. Only
          // this box sticks (main.css `.linenumber`), so a long line scrolled
          // sideways loses ~56px under the numbers, not the whole margin. 3.5em
          // holds four digits, and MAX_LINE_NUMBERED_LINES stops numbering
          // before a fifth is needed.
          minWidth: `calc(var(--paper-lead) + ${PAPER_GUTTER_EM})`,
          boxSizing: 'border-box',
          paddingLeft: 'var(--paper-lead)',
          paddingRight: '1.35em',
          textAlign: 'right',
          // Ink, slant and weight come from the `react-syntax-highlighter-line-
          // number` entry in codeTheme.ts (comment ink, upright, 400), faded to a
          // gutter in `codeStyle` above — never with `opacity` (see there).
          userSelect: 'none',
          // A gutter is the one place where digit alignment is the whole job.
          fontVariantNumeric: 'tabular-nums',
        }}
        customStyle={{
          margin: 0,
          // The inline edges come from the paper CSS variables (main.css,
          // `.br-paper`): unnumbered code starts on the column edge; numbered code
          // starts at the scroller's edge because each line carries the margin.
          padding: numbered
            ? '28px var(--paper-gutter) 48px 0'
            : '28px var(--paper-gutter) 48px var(--paper-inset)',
          minHeight: '100%',
          width: 'max-content',
          minWidth: '100%',
          boxSizing: 'border-box',
          background: 'transparent',
          // ⚠ Load-bearing. The theme's `pre` entry sets `overflow: auto`, which
          // makes this div a scroll container that never scrolls — and a sticky
          // gutter sticks to its NEAREST scroll container, so it would ride along
          // with the text. The paper scroller is the one that scrolls.
          overflow: 'visible',
        }}
        codeTagProps={{
          style: {
            fontFamily: CODE_FONT,
            whiteSpace: 'pre',
          },
        }}
      >
        {stripTrailingNewline(text)}
      </SyntaxHighlighter>
    </div>
  );
}

function CopyButton({ text }: { text: string }) {
  // `failed` is shown, never swallowed: a refused write used to leave the label
  // alone, so a Copy that copied nothing looked exactly like one that worked.
  // Icon-only now (spec 3.8): Copy, then Check; a failure turns the glyph into a
  // warning and its tooltip says what to do. The outcome is also announced.
  const [outcome, markOutcome] = useTransientValue<'copied' | 'failed'>(1600);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const failed = outcome === 'failed';

  return (
    <>
      <PreviewIconButton
        ref={buttonRef}
        compact
        label={PREVIEW_COPY.copy}
        hint={failed ? PREVIEW_COPY.copyFailed : undefined}
        data-testid="artifact-copy"
        data-outcome={outcome ?? undefined}
        onClick={async () => {
          // The shared path (utils/clipboard.ts): retried, then copied through the
          // document's own selection, whose hidden textarea goes in the bar.
          const copied = await copyToClipboard(text, buttonRef.current?.parentElement ?? null);
          markOutcome(copied ? 'copied' : 'failed');
        }}
        className={cn(failed ? 'text-text-warning' : 'text-text-muted')}
      >
        {outcome === 'copied' ? (
          <Check className="h-3.5 w-3.5" aria-hidden="true" />
        ) : failed ? (
          <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
        ) : (
          <Copy className="h-3.5 w-3.5" aria-hidden="true" />
        )}
      </PreviewIconButton>
      <span role="status" data-testid="artifact-copy-status" className="sr-only">
        {outcome === 'copied' ? PREVIEW_COPY.copied : failed ? PREVIEW_COPY.copyFailedShort : ''}
      </span>
    </>
  );
}

/**
 * The view switch every renderable kind shares (spec 2.6, 3.8): the one
 * `SegmentedControl`, never a bespoke bordered pair.
 */
function ViewSwitch<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: ReadonlyArray<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <SegmentedControl
      aria-label={PREVIEW_COPY.viewLabel}
      data-testid="artifact-view-switch"
      value={value}
      onValueChange={onChange}
      options={options}
      className="br-preview-view-switch"
    />
  );
}

// A written `.md` report and a written `.csv` table are the agent's output, not
// its source code — showing raw markup would make the user read the syntax to
// find the content. Both stay one click from the raw text. Everything else is a
// script, and gets highlighted and line-numbered.
//
// PAPER. Every branch below sits on `.br-paper` — the page ground
// (`--background-default`), which is byte-for-byte the ground an Auto
// Visualiser chart paints, so a report, its table and its figure read as one
// family of surface. The scroller is a size container and the content column is
// the chat measure (760px) centred in it; content that is naturally wider (a
// wide table, a long code line) keeps the column's LEFT edge and runs on to the
// right, so every kind shares one left edge at any panel width. The bar above
// (PreviewBar) starts on that same edge.
function TextFilePreview({
  file,
  resolvedTheme,
  onOpenArtifact,
  sourceLine,
}: {
  file: Extract<ArtifactFilePreview, { kind: 'text' | 'html' }>;
  resolvedTheme: 'light' | 'dark';
  onOpenArtifact?: (artifact: ArtifactSource) => void;
  sourceLine?: number;
}) {
  const markdown = isMarkdownPath(file.path);
  const delimited = isDelimitedPath(file.path);
  const html = file.kind === 'html';
  const renderable = markdown || delimited || html;
  const [showRaw, setShowRaw] = useState(sourceLine !== undefined);
  useEffect(() => {
    if (sourceLine !== undefined) setShowRaw(true);
  }, [sourceLine]);
  // Each view keeps its own scroll position: switching back lands where you were.
  const scrollerRef = useRef<HTMLDivElement>(null);
  const scrollByViewRef = useRef<{ raw: number; rendered: number }>({ raw: 0, rendered: 0 });
  const viewSwitchedRef = useRef(false);
  const switchView = (raw: boolean) => {
    if (raw === showRaw) return;
    viewSwitchedRef.current = true;
    const scroller = scrollerRef.current;
    if (scroller) scrollByViewRef.current[showRaw ? 'raw' : 'rendered'] = scroller.scrollTop;
    setShowRaw(raw);
  };
  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    if (scroller) scroller.scrollTop = scrollByViewRef.current[showRaw ? 'raw' : 'rendered'];
  }, [showRaw]);

  const lineCount = useMemo(() => countLines(file.text), [file.text]);
  const showingCode = showRaw || !renderable;
  // Parsed once: the table renders these rows and the bar states their count.
  const tableRows = useMemo(
    () =>
      delimited
        ? parseDelimitedTable(file.text, extensionFromPath(file.path) === 'tsv' ? '\t' : ',')
        : null,
    [delimited, file.path, file.text]
  );

  const code = (
    <CodeBlock
      text={file.text}
      language={languageForText(file.path, file.mimeType, file.text)}
      resolvedTheme={resolvedTheme}
      sourceLine={sourceLine}
    />
  );

  const totalRows = tableRows && tableRows.length > 0 ? tableRows.length - 1 : null;
  const hiddenRows = totalRows !== null ? Math.max(0, totalRows - MAX_TABLE_ROWS) : 0;
  const countText = showingCode
    ? `${lineCount.toLocaleString()} line${lineCount === 1 ? '' : 's'}`
    : totalRows !== null
      ? hiddenRows > 0
        ? PREVIEW_COPY.rowsShown(MAX_TABLE_ROWS, totalRows)
        : `${totalRows.toLocaleString()} row${totalRows === 1 ? '' : 's'}`
      : null;
  const lineNotice =
    showingCode &&
    sourceLine !== undefined &&
    (sourceLine > lineCount || lineCount > MAX_LINE_NUMBERED_LINES)
      ? sourceLine > lineCount
        ? PREVIEW_COPY.lineOutsideFile
        : PREVIEW_COPY.lineTooLarge
      : null;
  const viewOptions = [
    { value: 'rendered' as const, label: delimited ? PREVIEW_COPY.table : PREVIEW_COPY.preview },
    { value: 'raw' as const, label: PREVIEW_COPY.raw },
  ];

  return (
    <div className="br-paper flex h-full min-h-0 flex-col">
      <PreviewBarMeta>
        {/* The count yields first, and whole: it never pushes the name or the
            controls out of the bar (main.css, `.br-paper-strip-count`). */}
        {countText && (
          <span data-testid="artifact-strip-count" className="br-paper-strip-count tabular-nums">
            <span>{countText}</span>
          </span>
        )}
        {hiddenRows > 0 && !showingCode && (
          <InfoTip
            label={countText ?? PREVIEW_COPY.table}
            help={PREVIEW_COPY.rowsCapHint(hiddenRows)}
          />
        )}
        {lineNotice && (
          <span role="status" className="br-preview-bar__notice">
            {lineNotice}
          </span>
        )}
      </PreviewBarMeta>
      <PreviewBarActions>
        <CopyButton text={file.text} />
        {renderable && (
          <ViewSwitch
            value={showRaw ? 'raw' : 'rendered'}
            options={viewOptions}
            onChange={(next) => switchView(next === 'raw')}
          />
        )}
      </PreviewBarActions>
      {/* One ground for every view. The code view used to switch to
          --background-code here; on paper it does not need to, because every
          family's syntax palette is ALSO measured against --background-default
          (scripts/generate-themes.mjs, "paper ground"), and in light that ground
          only raises the ratios. */}
      <div
        ref={scrollerRef}
        data-preview-scroller=""
        className="br-paper-scroll min-h-0 flex-1 overflow-auto"
        data-view={showingCode ? 'code' : markdown ? 'prose' : html ? 'html' : 'table'}
      >
        {/* Keyed by view, so Preview ↔ Raw is a short fade of the incoming view
            (preview-panel.css) rather than a hard swap. */}
        <div
          key={showingCode ? 'raw' : 'rendered'}
          className="br-preview-view"
          data-enter={viewSwitchedRef.current ? '' : undefined}
        >
          {showingCode ? (
            code
          ) : markdown ? (
            <MarkdownDocument text={file.text} path={file.path} onOpenArtifact={onOpenArtifact} />
          ) : html ? (
            // Same sandbox + theme injection as the figure preview above. `allow-popups`
            // is withheld so the framed HTML can't window.open() into a real BrowserWindow
            // that would inherit the preload IPC bridge.
            <iframe
              name="biorouter-artifact-preview"
              key={file.preparedHtml ?? file.text}
              aria-label={file.title}
              srcDoc={injectArtifactBrowserCsp(
                withPreviewSizeReporting(
                  withPreviewActivityTracking(
                    withHostTheme(file.preparedHtml ?? file.text, resolvedTheme)
                  )
                )
              )}
              sandbox="allow-scripts allow-downloads"
              data-loaded="false"
              onLoad={markFrameLoaded}
              className="br-preview-frame h-full w-full bg-white"
            />
          ) : (
            <DelimitedTable rows={tableRows ?? []} maxRows={MAX_TABLE_ROWS} />
          )}
        </div>
      </div>
    </div>
  );
}
