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

export function Dropzone({ onFiles, onPathPickRequested, onPasteText }: Props) {
  const [dragging, setDragging] = useState(false);
  const [chooserOpen, setChooserOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  // Counter-based drag tracking prevents flicker when cursor moves over child elements.
  const dragCounterRef = useRef(0);

  const onDrop = useCallback(
    (e: React.DragEvent) => {
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

  function openChooser() {
    setChooserOpen(true);
  }

  return (
    <>
      {/* The drop target. A press anywhere on it opens the chooser, as it always
          has; the keyboard reaches the same thing through the label's own button,
          because a `role="button"` box cannot also hold the "Paste text" button
          and the InfoTip (an interactive control inside another). */}
      <div
        data-testid="knowledge-ingest-dropzone"
        onClick={openChooser}
        onDragEnter={(e) => {
          e.preventDefault();
          dragCounterRef.current++;
          setDragging(true);
        }}
        onDragOver={(e) => {
          e.preventDefault();
        }}
        onDragLeave={(e) => {
          e.preventDefault();
          dragCounterRef.current--;
          if (dragCounterRef.current === 0) setDragging(false);
        }}
        onDrop={onDrop}
        /* ⚠ **`flex-1` here is what closes the rail's empty middle, and it is a
           FUNCTIONAL fix rather than a cosmetic one.** The body packs its
           children at the top and the digest footer is pinned at the bottom, so
           an unstaged rail showed a tall hole between them. Letting the DROP
           TARGET absorb that slack means the target is largest exactly when you
           have nothing staged and are about to drop something, and shrinks back
           toward its floor as staged rows claim the space. `min-h` is that
           floor. */
        data-dragging={dragging ? 'true' : undefined}
        className="br-knowledge-dropzone relative flex min-h-[7.125rem] flex-1 cursor-pointer flex-col items-center justify-center gap-1 rounded-container px-4 py-4 text-center"
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
        <button
          type="button"
          className="br-knowledge-drop-target flex flex-col items-center gap-2 rounded-element px-2 py-1"
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
