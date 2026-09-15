import { useEffect, useRef, type RefObject } from 'react';
import { ARTIFACT_PANEL_ATTR } from '../../utils/tabCycle';
import type { ArtifactSource } from './artifactTypes';
import { basenameFromPath } from './artifactUtils';
import {
  registerPanelAccess,
  type PanelAccessor,
  type PanelDescriptor,
  type PanelTextSnapshot,
} from './panelAccessRegistry';
import { createPdfWorker } from '../../utils/pdfCompat';
import type { LiveBrowserShare } from './WebPagePreview';

/** Enough to be useful, small enough not to swamp a turn's context. */
const DEFAULT_TEXT_LIMIT = 20_000;

function artifactIdentity(artifact: ArtifactSource): string {
  if (artifact.kind === 'file') return `file:${artifact.path}`;
  if (artifact.kind === 'externalUrl') return `url:${artifact.url}`;
  if (artifact.kind === 'html') {
    return `html:${artifact.title}:${artifact.html.length}:${artifact.html.slice(0, 80)}`;
  }
  return `resource:${artifact.resource.uri}:${artifact.resource.mimeType ?? ''}`;
}

function accessIdentity(
  artifact: ArtifactSource | null,
  isOpen: boolean,
  liveBrowserShare: LiveBrowserShare | null | undefined,
  fileSourceRevision: string | null | undefined
): string {
  if (!isOpen || !artifact) return 'closed';
  const identity = artifactIdentity(artifact);
  if (artifact.kind === 'file') return `${identity}:${fileSourceRevision || 'not-rendered'}`;
  if (artifact.kind !== 'externalUrl') return identity;
  return liveBrowserShare
    ? `${identity}:${liveBrowserShare.viewId}:${liveBrowserShare.state.url}:${liveBrowserShare.state.sourceRevision}`
    : `${identity}:not-shared`;
}

async function readPdfText(data: ArrayBuffer, maxChars: number) {
  const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs');
  const worker = typeof Worker === 'undefined' ? null : createPdfWorker();
  if (worker) pdfjs.GlobalWorkerOptions.workerPort = worker;
  const task = pdfjs.getDocument({ data: new Uint8Array(data.slice(0)) });
  try {
    const document = await task.promise;
    let text = '';
    let truncated = document.numPages > 200;
    for (let pageNumber = 1; pageNumber <= Math.min(document.numPages, 200); pageNumber += 1) {
      const page = await document.getPage(pageNumber);
      const content = await page.getTextContent();
      const pageText = content.items
        .map((item) => ('str' in item ? item.str : ''))
        .filter(Boolean)
        .join(' ');
      text += `${text ? '\n\n' : ''}[Page ${pageNumber}]\n${pageText}`;
      page.cleanup();
      if (text.length > maxChars) {
        truncated = true;
        break;
      }
    }
    return { text: text.slice(0, maxChars), truncated };
  } finally {
    await task.destroy();
    worker?.terminate();
    if (pdfjs.GlobalWorkerOptions.workerPort === worker) {
      pdfjs.GlobalWorkerOptions.workerPort = null;
    }
  }
}

/**
 * Publishes this chat's artifact panel to the agent.
 *
 * Called from the chat surface rather than from `ArtifactViewer`, because this
 * needs both the panel's content *and* the session it belongs to, and only the
 * chat has both. Read-only transcripts deliberately do not call it: an agent
 * reading a saved session's panel would be reading a different conversation's
 * screen.
 */
export function useArtifactPanelAccess({
  sessionId,
  artifact,
  isOpen,
  tabCount,
  liveBrowserShare,
  panelRootRef,
  fileSourceRevision,
}: {
  sessionId: string | null | undefined;
  artifact: ArtifactSource | null;
  isOpen: boolean;
  tabCount?: number;
  /** Set when the panel is showing a live page, so we capture the right contents. */
  liveBrowserShare?: LiveBrowserShare | null;
  panelRootRef: RefObject<HTMLElement | null>;
  /** Revision of the bytes currently rendered in the file preview. */
  fileSourceRevision?: string | null;
}) {
  // Held in a ref so the accessor closes over *current* values without the
  // registry churning on every render.
  const nextAccessIdentity = accessIdentity(artifact, isOpen, liveBrowserShare, fileSourceRevision);
  const stateRef = useRef({
    artifact,
    isOpen,
    tabCount,
    liveBrowserShare,
    fileSourceRevision,
    accessIdentity: nextAccessIdentity,
    generation: 0,
  });
  const generation =
    stateRef.current.accessIdentity === nextAccessIdentity
      ? stateRef.current.generation
      : stateRef.current.generation + 1;
  stateRef.current = {
    artifact,
    isOpen,
    tabCount,
    liveBrowserShare,
    fileSourceRevision,
    accessIdentity: nextAccessIdentity,
    generation,
  };

  useEffect(() => {
    if (!sessionId) return;

    const describe = (): PanelDescriptor => {
      const {
        artifact: current,
        isOpen: open,
        tabCount: tabs,
        liveBrowserShare: liveShare,
        fileSourceRevision: renderedFileRevision,
      } = stateRef.current;
      if (!open || !current) return { open: false };
      const liveState = current.kind === 'externalUrl' ? liveShare?.state : undefined;
      return {
        open: true,
        kind: current.kind === 'externalUrl' ? 'webPage' : current.kind,
        title: liveState?.title || current.title,
        locator:
          current.kind === 'file'
            ? current.path
            : current.kind === 'externalUrl'
              ? liveState?.url || current.url
              : undefined,
        sourceRevision:
          current.kind === 'file' ? renderedFileRevision || undefined : liveState?.sourceRevision,
        tabCount: tabs,
      };
    };

    const readText = async (maxChars: number): Promise<PanelTextSnapshot | null> => {
      const limit = Number.isFinite(maxChars)
        ? Math.min(40_000, Math.max(0, Math.floor(maxChars || DEFAULT_TEXT_LIMIT)))
        : DEFAULT_TEXT_LIMIT;
      const {
        artifact: current,
        isOpen: open,
        liveBrowserShare: liveShare,
        fileSourceRevision: renderedFileRevision,
        accessIdentity: sourceIdentity,
        generation: sourceGeneration,
      } = stateRef.current;
      if (!open || !current) return null;

      const sourceIsCurrent = () => {
        const latest = stateRef.current;
        return (
          latest.isOpen &&
          latest.artifact !== null &&
          latest.accessIdentity === sourceIdentity &&
          latest.generation === sourceGeneration
        );
      };

      const clip = (text: string) => ({
        text: text.slice(0, limit),
        truncated: text.length > limit,
      });

      if (current.kind === 'externalUrl') {
        // A live page's text lives in a different WebContents, so it has to be
        // read through the view rather than from anything in this renderer.
        if (!liveShare) return null;
        const page = await window.electron?.embeddedBrowser?.readText(liveShare.viewId, limit);
        if (!page) return null;
        if (
          !sourceIsCurrent() ||
          page.url !== liveShare.state.url ||
          page.sourceRevision !== liveShare.state.sourceRevision
        ) {
          return null;
        }
        return {
          kind: 'webPage',
          title: page.title || current.title,
          locator: page.url,
          sourceRevision: page.sourceRevision,
          text: page.text,
          truncated: page.truncated,
        };
      }

      if (current.kind === 'html') {
        const document = new DOMParser().parseFromString(current.html, 'text/html');
        document
          .querySelectorAll('script, style, template, noscript')
          .forEach((node) => node.remove());
        return {
          kind: 'html',
          title: current.title,
          ...clip(document.body?.innerText || document.body?.textContent || ''),
        };
      }

      if (current.kind === 'file') {
        const preview = await window.electron?.readArtifactFile(current.path);
        if (!preview || !('kind' in preview) || !sourceIsCurrent()) return null;
        if (
          !renderedFileRevision ||
          !('revision' in preview) ||
          preview.revision !== renderedFileRevision
        ) {
          return null;
        }
        if (preview.kind === 'text' || preview.kind === 'html') {
          return {
            kind: preview.kind,
            title: current.title || basenameFromPath(current.path),
            locator: current.path,
            sourceRevision: preview.revision,
            ...clip(preview.text),
          };
        }
        if (preview.kind === 'document') {
          const extracted =
            preview.format === 'pdf'
              ? await readPdfText(preview.data, limit)
              : {
                  text: (preview.extractedText ?? '').slice(0, limit),
                  truncated:
                    preview.textTruncated === true || (preview.extractedText?.length ?? 0) > limit,
                };
          if (!extracted.text || !sourceIsCurrent()) return null;
          if (preview.format === 'pdf') {
            const confirmation = await window.electron?.readArtifactFile(current.path);
            if (
              !sourceIsCurrent() ||
              !preview.revision ||
              confirmation?.kind !== 'document' ||
              confirmation.format !== 'pdf' ||
              confirmation.revision !== preview.revision
            ) {
              return null;
            }
          }
          return {
            kind: preview.format,
            title: current.title || basenameFromPath(current.path),
            locator: current.path,
            sourceRevision: preview.revision,
            ...extracted,
          };
        }
        // Images, documents and binaries have no text to give. Saying so is
        // more useful than returning an empty string the model would read as
        // "the file is blank".
        return null;
      }

      return null;
    };

    const capture = async () => {
      const { artifact: current, isOpen: open, liveBrowserShare: liveShare } = stateRef.current;
      if (!open || !current) return null;
      const sourceIdentity = artifactIdentity(current);

      // ⚠ A live page is a separate `WebContents` — a sibling native layer, not
      // part of this document — so a window capture would return the panel's
      // chrome with a hole where the page is. It has to capture itself.
      if (current.kind === 'externalUrl') {
        if (!liveShare) return null;
        const shot = await window.electron?.embeddedBrowser?.capture(liveShare.viewId);
        if (!shot) return null;
        const latest = stateRef.current;
        const stillShared =
          latest.isOpen &&
          latest.artifact === current &&
          artifactIdentity(latest.artifact) === sourceIdentity &&
          latest.liveBrowserShare?.viewId === liveShare.viewId &&
          latest.liveBrowserShare.state.sourceRevision === liveShare.state.sourceRevision &&
          shot.sourceRevision === liveShare.state.sourceRevision;
        if (!stillShared) {
          window.electron?.deleteTempFile(shot.path);
          return null;
        }
        return shot;
      }

      // Everything else lives in this document, including the sandboxed
      // `srcdoc` frames that no DOM-walking screenshot library can enter —
      // `capturePage` is a compositor grab, so it sees straight through.
      const panel = panelRootRef.current?.querySelector<HTMLElement>(`[${ARTIFACT_PANEL_ATTR}]`);
      if (!panel) return null;
      const rect = panel.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return null;
      // ⚠ Optional on the METHOD, not only the bridge: a `biorouter serve`
      // browser's bridge has no `captureRegion` (captureOnBrowser.ts), and a
      // throw here reached the agent as the tool's result.
      const shot = await window.electron?.captureRegion?.({
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
        label: 'panel',
        containment: {
          x: rect.left,
          y: rect.top,
          width: rect.width,
          height: rect.height,
        },
      });
      if (!shot) return null;
      const latest = stateRef.current;
      if (
        !latest.isOpen ||
        latest.artifact !== current ||
        artifactIdentity(latest.artifact) !== sourceIdentity
      ) {
        window.electron?.deleteTempFile(shot.path);
        return null;
      }
      return shot;
    };

    const accessor: PanelAccessor = { describe, readText, capture };
    return registerPanelAccess(sessionId, accessor);
  }, [panelRootRef, sessionId]);
}
