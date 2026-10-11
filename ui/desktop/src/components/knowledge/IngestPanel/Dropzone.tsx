import { useCallback, useRef, useState } from 'react';
import { Clipboard, FileStack, FolderTree, Upload } from '../../icons/app-icons';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { InfoTip } from '../../ui/info-tip';
import { knowledgeCopy } from '../copy';
import type { StagedFileCandidate } from './fileValidation';

interface Props {
  onFiles: (files: StagedFileCandidate[]) => void | Promise<void>;
  onPathPickRequested: () => void | Promise<void>;
  /** Summon the paste box. "Paste text" lives inside the dropzone (spec 3.12). */
  onPasteText: () => void;
  /**
   * Something is being dragged over the rail (`useFileDrop`). The box does not own the drop any
   * more, the whole rail does, so the box only shows the answer.
   */
  dragging?: boolean;
}

function labelFromPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function tryParseLocalPath(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.startsWith('#')) {
    return null;
  }

  if (trimmed.startsWith('file://')) {
    try {
      return decodeURIComponent(new URL(trimmed).pathname);
    } catch {
      return null;
    }
  }

  if (trimmed.startsWith('/') || /^[A-Za-z]:[\\/]/.test(trimmed)) {
    return trimmed;
  }

  return null;
}

function getDroppedPathCandidates(e: React.DragEvent): StagedFileCandidate[] {
  const candidates = new Map<string, StagedFileCandidate>();
  const uriList = e.dataTransfer.getData('text/uri-list');
  const plainText = e.dataTransfer.getData('text/plain');

  for (const raw of `${uriList}\n${plainText}`.split('\n')) {
    const parsed = tryParseLocalPath(raw);
    if (!parsed) {
      continue;
    }
    candidates.set(parsed, { path: parsed, label: labelFromPath(parsed) });
  }

  return [...candidates.values()];
}

/**
 * The Sources rail's drop target: the WHOLE rail, not the box in it (V3-12, kept by R-33).
 *
 * The box fills the rail's free height again (R-33), but it yields that room to the paste box and
 * the staged list as they appear, so a file let go on a staged row, on the paste box's frame or on
 * the footer must still stage. `IngestPanel` spreads `dropProps` on the rail's root and hands
 * `dragging` to the box and to the rail: the box takes its drag fill and edge, and the rail draws
 * an accent edge around itself (`.br-knowledge-rail`), so a held file gets an answer even when a
 * long staged list has scrolled the box out of view. The rail carries no fill.
 *
 * ⚠ **Text fields are left alone.** A drag that starts or lands inside the paste box's fields is a
 * text drag; claiming it would cancel the browser's own insertion.
 */
export function useFileDrop(onFiles: Props['onFiles']) {
  const [dragging, setDragging] = useState(false);
  // Counter-based drag tracking prevents flicker when cursor moves over child elements.
  const dragCounterRef = useRef(0);

  const inTextField = (e: React.DragEvent) =>
    e.target instanceof Element && e.target.closest('input, textarea, [contenteditable]') !== null;

  const onDrop = useCallback(
    (e: React.DragEvent) => {
      if (inTextField(e)) return;
      e.preventDefault();
      dragCounterRef.current = 0;
      setDragging(false);
      const droppedFiles = Array.from(e.dataTransfer.files).map((file) => ({ file }));
      const droppedPaths = getDroppedPathCandidates(e);
      const candidates = [...droppedFiles, ...droppedPaths];
      if (candidates.length > 0) void onFiles(candidates);
    },
    [onFiles]
  );

  const dropProps = {
    onDragEnter: (e: React.DragEvent) => {
      if (inTextField(e)) return;
      e.preventDefault();
      dragCounterRef.current++;
      setDragging(true);
    },
    onDragOver: (e: React.DragEvent) => {
      if (inTextField(e)) return;
      e.preventDefault();
    },
    onDragLeave: (e: React.DragEvent) => {
      if (inTextField(e)) return;
      e.preventDefault();
      dragCounterRef.current = Math.max(0, dragCounterRef.current - 1);
      if (dragCounterRef.current === 0) setDragging(false);
    },
    onDrop,
  };

  return { dragging, dropProps };
}

export function Dropzone({ onFiles, onPathPickRequested, onPasteText, dragging = false }: Props) {
  const [chooserOpen, setChooserOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  function openChooser() {
    setChooserOpen(true);
  }

  return (
    <>
      {/* The box a person aims at. A press anywhere on it opens the chooser, as it always has; the
          keyboard reaches the same thing through the label's own button, because a
          `role="button"` box cannot also hold the "Paste text" button and the InfoTip (an
          interactive control inside another).

          ⚠ **Only a press INSIDE the box counts.** React bubbles a synthetic
          event up the React tree, not the DOM tree, so a click on the InfoTip's
          floating text (portaled out of this box) would otherwise arrive here
          and open the chooser over the tip being read.

          ⚠ **It fills the rail's free height, and it does not own the drop** (R-33 over V3-12).
          V3-12 cut it to its content (about 160px), which left a small box over an empty rail; the
          owner wants the side-panel scale back. Its size is authored on `.br-knowledge-dropzone`
          (grow into the scroller's free height, yield it to the paste box and the staged list, a
          240px floor), not written here, because a sizing utility that fails to generate would put
          the small box back silently. The drop is the rail's (`useFileDrop`). */}
      <div
        data-testid="knowledge-ingest-dropzone"
        onClick={(e) => {
          if (e.currentTarget.contains(e.target as Node)) openChooser();
        }}
        data-dragging={dragging ? 'true' : undefined}
        className="br-knowledge-dropzone relative flex cursor-pointer flex-col items-center justify-center gap-1 rounded-container px-4 py-4 text-center"
      >
        <input
          data-testid="knowledge-ingest-file-input"
          ref={inputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            const files = e.target.files ? Array.from(e.target.files) : [];
            if (files.length > 0) {
              void onFiles(files.map((file) => ({ file })));
            }
            e.target.value = '';
          }}
        />
        {/* A 32px `rounded-element` plate with a 16px icon: the dropzone's own
            mark, deliberately smaller than `EmptyState`'s 48px plate, because
            this is a control in a 300px rail and not a page's empty state. */}
        {/* Named for what a press does (it opens the chooser), not only for the drag its words
            describe; the name starts with those words (copy.ts, `dropButtonName`). */}
        <button
          type="button"
          aria-label={knowledgeCopy.ingest.dropButtonName}
          className="flex flex-col items-center gap-2 rounded-element px-2 py-1"
          onClick={(e) => {
            e.stopPropagation();
            openChooser();
          }}
        >
          <span
            aria-hidden="true"
            className="br-knowledge-drop-plate flex h-8 w-8 items-center justify-center rounded-element"
          >
            <Upload className="h-icon-row w-icon-row" />
          </span>
          <span className="text-label">{knowledgeCopy.ingest.dropLabel}</span>
        </button>
        <div className="flex items-center justify-center gap-1 text-supporting text-text-muted">
          <span>{knowledgeCopy.ingest.dropHint}</span>
          <InfoTip
            label={knowledgeCopy.ingest.dropInfoLabel}
            help={knowledgeCopy.ingest.dropInfo}
            side="bottom"
            align="center"
          />
        </div>
        {/* "Paste text" is a second way IN, so it sits with the first one rather
            than as a full-width slab under the dropzone (spec 3.12). */}
        <Button
          data-testid="knowledge-ingest-paste-text"
          type="button"
          variant="ghost"
          size="sm"
          className="mt-1"
          onClick={(e) => {
            e.stopPropagation();
            onPasteText();
          }}
        >
          <Clipboard aria-hidden="true" />
          {knowledgeCopy.ingest.pasteText}
        </Button>
      </div>

      <ModalShell
        open={chooserOpen}
        onOpenChange={setChooserOpen}
        size="md"
        title={knowledgeCopy.ingest.chooserTitle}
        subtitle={knowledgeCopy.ingest.chooserSubtitle}
        bodyClassName="pb-4"
      >
        <div className="grid gap-2 sm:grid-cols-2">
          <button
            data-testid="knowledge-ingest-browse-files"
            type="button"
            onClick={() => {
              setChooserOpen(false);
              inputRef.current?.click();
            }}
            className="biorouter-modal-row flex items-center gap-3 rounded-element px-4 py-3 text-left transition-colors tint-interactive"
          >
            <FileStack className="h-icon-row w-icon-row shrink-0 text-text-muted" aria-hidden />
            <span className="text-label">{knowledgeCopy.ingest.chooseFiles}</span>
          </button>

          <button
            data-testid="knowledge-ingest-browse-path"
            type="button"
            onClick={() => {
              setChooserOpen(false);
              void onPathPickRequested();
            }}
            className="biorouter-modal-row flex items-center gap-3 rounded-element px-4 py-3 text-left transition-colors tint-interactive"
          >
            <FolderTree className="h-icon-row w-icon-row shrink-0 text-text-muted" aria-hidden />
            <span className="text-label">{knowledgeCopy.ingest.chooseFolder}</span>
          </button>
        </div>
      </ModalShell>
    </>
  );
}
