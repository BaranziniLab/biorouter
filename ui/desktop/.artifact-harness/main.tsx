// Browser harness for the artifact side panel.
//
// Mounts the real `ArtifactViewer` against the real backend output (fixtures
// produced by the Rust tools) with only the Electron IPC bridge stubbed, so what
// renders here is what renders in the app. Driven by agent-browser.
//
//   npx vite --config .artifact-harness/vite.config.mts --port 5199
import { StrictMode, useLayoutEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import ArtifactViewer from '../src/components/artifacts/ArtifactViewer';
import type { ArtifactSource } from '../src/components/artifacts/artifactTypes';
import {
  useArtifactPanel,
  type ArtifactPanelPresentedChange,
} from '../src/components/artifacts/useArtifactPanel';
import { ThemeProvider } from '../src/contexts/ThemeContext';
import { useColumnGlide } from '../src/hooks/useColumnGlide';
import { DUR } from '../src/styles/motion';
import { IMAGE_MIME_TYPES } from '../src/utils/imageFormats';
import './harness.css';

// Text MIME types only. Images come from the SHARED table the main process
// uses, so the harness cannot claim support the app does not have — which is
// the whole point of this file existing.
const MIME: Record<string, string> = {
  csv: 'text/csv',
  html: 'text/html',
  md: 'text/markdown',
  r: 'text/x-r',
  tsv: 'text/tab-separated-values',
};

const OFFICE_MIME: Record<string, string> = {
  pdf: 'application/pdf',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
};

const extOf = (path: string) => path.split('.').pop()!.toLowerCase();

// A git repository listing is the only place the panel renders the branch ref
// and the status legend, and D-33 splits those two across the font boundary
// (ref = mono, "Modified" = sans). Neither is reachable from a file fixture, so
// the sweep could not see them at all without this. Shaped exactly like the main
// process's `readArtifactFile` gitDirectory reply.
const GIT_DIR_PATH = '/w/repo';

// One file per git status, plus a nested directory so the tree's indent is
// exercised. Each name is a fixture the harness serves, so selecting a row shows
// a real nested preview rather than a 404. `parentPath` is relative and ''
// at the root, and a child's is its parent's RELATIVE path — the shape
// artifactDirectory.ts's `visit('', …)` produces. An absolute parentPath here
// renders "This folder is empty": the tree roots at appendChildren('').
const GIT_ENTRIES: {
  name: string;
  relativePath: string;
  parentPath: string;
  isDirectory: boolean;
  status: 'untracked' | 'modified' | 'staged' | 'committed' | 'pushed';
}[] = [
  {
    name: 'analysis.R',
    relativePath: 'analysis.R',
    parentPath: '',
    isDirectory: false,
    status: 'modified',
  },
  {
    name: 'summarize.py',
    relativePath: 'summarize.py',
    parentPath: '',
    isDirectory: false,
    status: 'untracked',
  },
  {
    name: 'config.json',
    relativePath: 'config.json',
    parentPath: '',
    isDirectory: false,
    status: 'staged',
  },
  { name: 'R', relativePath: 'R', parentPath: '', isDirectory: true, status: 'committed' },
  {
    name: 'query.sql',
    relativePath: 'R/query.sql',
    parentPath: 'R',
    isDirectory: false,
    status: 'committed',
  },
  {
    name: 'report.md',
    relativePath: 'report.md',
    parentPath: '',
    isDirectory: false,
    status: 'pushed',
  },
];

const gitDirectoryFixture = () => ({
  kind: 'gitDirectory' as const,
  title: 'repo',
  path: GIT_DIR_PATH,
  branch: 'feat/redesign-ui-cohesion',
  found: true as const,
  entries: GIT_ENTRIES.map((entry) => ({
    ...entry,
    path: `${GIT_DIR_PATH}/${entry.relativePath}`,
  })),
});

async function readFixture(path: string) {
  if (path === GIT_DIR_PATH) return gitDirectoryFixture();
  const name = path.split('/').pop()!;
  const response = await fetch(`/fixtures/${name}`);
  const ext = extOf(name);
  // A missing fixture falls through to Vite's SPA fallback, which answers 200
  // with the harness's own index.html. The fixture route always answers
  // octet-stream, so text/html for a name that is not .html is that fallback:
  // treat it as the 404 it is, or the panel previews the harness page.
  const fellThrough =
    ext !== 'html' && (response.headers.get('content-type') ?? '').includes('text/html');
  if (!response.ok || fellThrough) {
    const status = fellThrough ? 404 : response.status;
    return { kind: 'error', title: name, path, error: `HTTP ${status}`, found: false };
  }
  const imageMime = IMAGE_MIME_TYPES[ext];
  const officeMime = OFFICE_MIME[ext];
  const mimeType = imageMime ?? officeMime ?? MIME[ext] ?? 'text/plain';

  if (officeMime) {
    const buffer = await response.arrayBuffer();
    return {
      kind: 'document',
      format: ext as 'pdf' | 'docx' | 'xlsx' | 'pptx',
      title: name,
      path,
      mimeType,
      data: buffer,
      size: buffer.byteLength,
      found: true,
    };
  }

  if (imageMime) {
    const buffer = await response.arrayBuffer();
    // TIFF is handed over as bytes, exactly as the main process does: the
    // renderer has to decode it before anything can be shown.
    if (imageMime === 'image/tiff') {
      return {
        kind: 'image',
        title: name,
        path,
        mimeType,
        bytes: buffer,
        size: buffer.byteLength,
        found: true,
      };
    }
    let binary = '';
    new Uint8Array(buffer).forEach((byte) => (binary += String.fromCharCode(byte)));
    return {
      kind: 'image',
      title: name,
      path,
      mimeType,
      dataUrl: `data:${mimeType};base64,${btoa(binary)}`,
      size: buffer.byteLength,
      found: true,
    };
  }

  // Anything the app would treat as an opaque binary, so the harness can show
  // the format-specific refusal card rather than only the happy paths.
  if (
    ['doc', 'ppt', 'xls', 'odt', 'ods', 'odp', 'rtf', 'pages', 'numbers', 'key', 'heic'].includes(
      ext
    )
  ) {
    const buffer = await response.arrayBuffer();
    return {
      kind: 'binary',
      title: name,
      path,
      mimeType: 'application/octet-stream',
      size: buffer.byteLength,
      found: true,
    };
  }

  const text = await response.text();
  return {
    kind: ext === 'html' ? 'html' : 'text',
    title: name,
    path,
    mimeType,
    text,
    size: text.length,
    found: true,
  };
}

// Only the IPC bridge is faked; every component below is the shipping one.
Object.defineProperty(window, 'electron', {
  configurable: true,
  value: {
    prepareArtifactHtml: async ({ html }: { html: string }) => ({ html }),
    readArtifactFile: readFixture,
    openExternal: async () => undefined,
    broadcastThemeChange: () => undefined,
    on: () => () => undefined,
    off: () => undefined,
    getConfig: () => ({}),
    // Region capture happens in the main process (a compositor grab, the only
    // thing that can see into a sandboxed frame). The harness reports the rect
    // instead so the overlay's arithmetic is visible in a real browser.
    captureRegion: async (rect: { x: number; y: number; width: number; height: number }) => {
      (window as unknown as { __lastCapture?: unknown }).__lastCapture = rect;
      return { path: '/tmp/harness-capture.png', width: rect.width, height: rect.height };
    },
    deleteTempFile: () => undefined,
    // The rest of the bridge the panel reaches: Open externally, Reveal in
    // Finder and an external page's live view. They record what they were asked,
    // so a sweep can assert it, and never leave the harness.
    openArtifactInBrowser: async (payload: unknown) => {
      (window as unknown as { __lastOpenInBrowser?: unknown }).__lastOpenInBrowser = payload;
    },
    openDirectoryInExplorer: async (target: string) => {
      (window as unknown as { __lastReveal?: unknown }).__lastReveal = target;
      return true;
    },
    getTempImage: async () => null,
    // The OS window grows only in Electron. A measuring main process may expose
    // `__harnessGrow` through its preload (contextBridge); a plain browser has
    // none and the grow resolves at once.
    ensureWindowContentWidth: async (width: number) =>
      (window as unknown as { __harnessGrow?: (w: number) => Promise<unknown> }).__harnessGrow?.(
        width
      ),
    // The live page is a native view in the app; here it is a placeholder that
    // answers like one, so "Open here", the toolbar and its states can be swept.
    embeddedBrowser: {
      isManagedAppUrl: async () => false,
      create: async (_viewId: string, url: string) => ({
        url,
        title: url,
        canGoBack: false,
        canGoForward: false,
        isLoading: false,
      }),
      setBounds: async () => undefined,
      setVisible: async () => undefined,
      navigate: async () => true,
      control: async () => undefined,
      readText: async () => null,
      capture: async () => null,
      clearData: async () => true,
      destroy: async () => undefined,
      onState: () => () => undefined,
    },
  },
});

const ARTIFACTS: { label: string; make: () => Promise<ArtifactSource> }[] = [
  {
    label: 'autovisualiser dashboard',
    make: async () => ({
      kind: 'html',
      title: 'Dashboard Tumour Vs Normal',
      html: await (await fetch('/fixtures/dashboard.html')).text(),
    }),
  },
  {
    label: 'agent drafter app',
    make: async () => ({
      kind: 'html',
      title: 'Agent Drafter Cohort Explorer',
      html: await (await fetch('/fixtures/agent-drafter-card.html')).text(),
    }),
  },
  {
    label: 'external page',
    make: async (): Promise<ArtifactSource> => ({
      kind: 'externalUrl',
      title: 'UCSF',
      url: 'https://www.ucsf.edu/',
    }),
  },
  {
    label: 'git repository',
    make: async (): Promise<ArtifactSource> => ({
      kind: 'file',
      title: 'repo',
      path: GIT_DIR_PATH,
    }),
  },
  // One entry per file kind the agent can create, so the whole preview surface can
  // be swept in a real browser (jsdom cannot see the Tailwind/Prism class clash).
  ...[
    'report.md',
    'genes.csv',
    'config.json',
    'pipeline.yaml',
    'sample.xml',
    'analysis.R',
    'summarize.py',
    'notebook.ipynb',
    'query.sql',
    'page.html',
    'theme.css',
    'run.sh',
    'notes.txt',
    'Cargo.toml',
    'lib.rs',
    'app.ts',
    'volcano.png',
    'test.pdf',
    'sample.docx',
    'FinancialSample.xlsx',
    'sample.pptx',
    // The formats this panel gained: three Chromium already decoded and the
    // panel refused for no reason, one that needs a real decoder, and two it
    // declines by name rather than with a shrug.
    'sample.avif',
    'sample.bmp',
    'sample.ico',
    'sample.svg',
    'sample.tiff',
    'report.doc',
    'deck.key',
  ].map((name) => ({
    label: name,
    make: async (): Promise<ArtifactSource> => ({ kind: 'file', title: name, path: `/w/${name}` }),
  })),
];

/**
 * PANEL MODE (`?mode=panel`). The default mode mounts `ArtifactViewer` already
 * open, under StrictMode, so its entrance never plays. This mode mounts it the
 * way the app does: `useArtifactPanel` drives a real `[data-preview-split]` grid
 * (the rung-2 CSS in main.css) beside a fake header, transcript and composer, and
 * the conversation glide (`useColumnGlide`) runs on the same `onPresentedChange`
 * the chat wires. Open, Close and Switch buttons sit in the fake header, so the
 * open, the switch crossfade, the close and a side and stack crossing can be
 * filmed (agent-browser `record_start`) and sampled frame by frame.
 *
 * Inline styles only, as in the rest of this file: Tailwind does not scan this
 * dot-directory, so a utility written here would never be generated.
 */
function PanelHarness() {
  const [index, setIndex] = useState(0);
  const presentedChangeRef = useRef<((change: ArtifactPanelPresentedChange) => void) | null>(null);
  const panel = useArtifactPanel({
    isMobile: false,
    // `?grow=1` lets the panel ask for a wider window, as the live chat does.
    allowWindowResize: new URLSearchParams(window.location.search).get('grow') === '1',
    onPresentedChange: (change) => presentedChangeRef.current?.(change),
  });
  const glide = useColumnGlide(panel.splitPaneRef);
  presentedChangeRef.current = (change) => {
    if (change.layout === 'side') glide.capture({ windowGrowing: change.windowGrowing });
  };
  const mounted = Boolean(panel.artifact);
  // While a preview is placed, main.css lays the split out as a grid and
  // flattens the column and body (`display: contents`). An inline `display`
  // would beat those rules, so the shell sets one only when nothing is placed.
  const laidOut = 'data-preview-layout' in panel.splitPaneProps;
  useLayoutEffect(() => {
    glide.playCaptured({ duration: mounted ? DUR.med : DUR.fastMax });
  }, [mounted, glide]);

  const files = ARTIFACTS.filter((entry) => entry.label.includes('.'));
  const openAt = async (next: number) => {
    const entry = files[((next % files.length) + files.length) % files.length];
    setIndex(next);
    await panel.openArtifact(await entry.make());
  };
  const button = (label: string, onClick: () => void, testId: string) => (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      style={{
        height: 28,
        padding: '0 10px',
        border: '1px solid var(--border-subtle)',
        borderRadius: 8,
        background: 'var(--background-default)',
        color: 'var(--text-default)',
        fontSize: 13,
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );

  return (
    <div
      data-testid="panel-host"
      style={{
        display: 'flex',
        height: '100vh',
        width: '100vw',
        background: 'var(--background-default)',
        color: 'var(--text-default)',
      }}
    >
      <div
        ref={panel.splitPaneRef}
        {...panel.splitPaneProps}
        style={{
          position: 'relative',
          display: laidOut ? undefined : 'flex',
          flex: 1,
          minWidth: 0,
          minHeight: 0,
          ...panel.splitPaneProps.style,
        }}
      >
        <div
          data-preview-area="column"
          style={{
            display: laidOut ? undefined : 'flex',
            flex: 1,
            minWidth: 0,
            flexDirection: 'column',
          }}
        >
          <div
            data-preview-area="body"
            style={{
              display: laidOut ? undefined : 'flex',
              flex: 1,
              minHeight: 0,
              flexDirection: 'column',
            }}
          >
            <div
              data-preview-area="header"
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                height: 44,
                padding: '0 16px',
                borderBottom: '1px solid var(--border-subtle)',
                flexShrink: 0,
              }}
            >
              <span style={{ fontSize: 14, marginRight: 'auto' }}>Harness chat</span>
              {button('Open', () => void openAt(index), 'panel-open')}
              {button('Switch', () => void openAt(index + 1), 'panel-switch')}
              {button('Close', () => panel.closePanel(), 'panel-close')}
            </div>
            <div data-preview-area="transcript" style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
              <div
                className="biorouter-chat-column"
                style={{ maxWidth: 760, margin: '0 auto', padding: '24px' }}
              >
                {Array.from({ length: 12 }, (_, i) => (
                  <p key={i} style={{ fontSize: 14, lineHeight: '21px', margin: '0 0 16px' }}>
                    {i % 2
                      ? 'The volcano plot shows 412 genes past the FDR threshold, most of them up in the tumour samples.'
                      : 'Open a figure, switch between files and close the panel to film how it moves.'}
                  </p>
                ))}
              </div>
            </div>
            <div data-preview-area="composer" style={{ flexShrink: 0, padding: '12px 24px 20px' }}>
              <div
                data-composer-shell=""
                style={{
                  maxWidth: 760,
                  margin: '0 auto',
                  height: 56,
                  border: '1px solid var(--border-subtle)',
                  borderRadius: 16,
                }}
              />
            </div>
          </div>
        </div>
        {panel.artifact && <ArtifactViewer {...panel.viewerProps} sessionId="harness-session" />}
      </div>
    </div>
  );
}

function Harness() {
  const [artifact, setArtifact] = useState<ArtifactSource | null>(null);
  const [active, setActive] = useState<string>('');

  const open = async (entry: (typeof ARTIFACTS)[number]) => {
    setActive(entry.label);
    setArtifact(await entry.make());
  };

  // The shell uses inline styles on purpose: Tailwind v4 does not scan
  // dot-directories, so utility classes written here would never be generated.
  // Everything inside ArtifactViewer still gets its classes from src/.
  return (
    <div style={{ display: 'flex', height: '100vh', width: '100vw' }}>
      <div style={{ width: 220, flexShrink: 0, borderRight: '1px solid #ddd', padding: 8 }}>
        {ARTIFACTS.map((entry) => (
          <button
            key={entry.label}
            type="button"
            data-testid={`open-${entry.label.replace(/\s+/g, '-')}`}
            onClick={() => open(entry)}
            style={{
              display: 'block',
              width: '100%',
              textAlign: 'left',
              padding: '8px 10px',
              marginBottom: 2,
              fontSize: 13,
              borderRadius: 6,
              border: 0,
              cursor: 'pointer',
              background: active === entry.label ? '#eee' : 'transparent',
            }}
          >
            {entry.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minWidth: 0, height: '100%' }} data-testid="panel-host">
        {artifact ? (
          <ArtifactViewer
            artifact={artifact}
            onClose={() => setArtifact(null)}
            onOpenArtifact={setArtifact}
            // A session id is what enables the annotate control — it is
            // chat-only, so a read-only transcript never shows it. The harness
            // stands in for a chat so the region-select can be swept here too.
            sessionId="harness-session"
          />
        ) : (
          <div style={{ padding: 24, fontSize: 13 }}>Pick an artifact.</div>
        )}
      </div>
    </div>
  );
}

// `?mode=panel` films the panel as the app mounts it; the default lists every
// fixture. Panel mode leaves StrictMode out: its simulated unmount cancels the
// entrance animation (usePreviewMotion's cleanup), which is the one thing that
// mode exists to show.
const panelMode = new URLSearchParams(window.location.search).get('mode') === 'panel';

createRoot(document.getElementById('root')!).render(
  panelMode ? (
    <ThemeProvider>
      <PanelHarness />
    </ThemeProvider>
  ) : (
    <StrictMode>
      <ThemeProvider>
        <Harness />
      </ThemeProvider>
    </StrictMode>
  )
);
