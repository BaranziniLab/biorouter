import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type ReactNode,
} from 'react';
import { Upload } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { filesCopy } from './copy';
import './files.css';

/** What a drop or a paste hands over. Never the bytes, never a path. */
export interface DroppedFiles {
  files: File[];
  /** At least one dropped item is a folder (read from the entry, not the name). */
  hasFolder: boolean;
}

/** The surface that accepts files dropped on a zone, and the channel it shares them in. */
export interface CrewDropTarget {
  channelName: string;
  onFiles(dropped: DroppedFiles): void;
}

interface DropZoneRegistry {
  register(target: CrewDropTarget): () => void;
}

const DropZoneContext = createContext<DropZoneRegistry | null>(null);

/**
 * Hand this surface's drop target to the nearest enclosing `CrewFileDropZone`, while mounted.
 * Returns whether there is one; a surface with none can wrap itself in its own zone. Pass
 * `null` while the surface cannot take files (an archived channel, an unverified view).
 */
export function useCrewDropTarget(target: CrewDropTarget | null): boolean {
  const registry = useContext(DropZoneContext);
  useEffect(() => (registry && target ? registry.register(target) : undefined), [registry, target]);
  return registry !== null;
}

function carriesFiles(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes('Files');
}

function readDrop(transfer: DataTransfer): DroppedFiles {
  const files = Array.from(transfer.files ?? []);
  const hasFolder = Array.from(transfer.items ?? []).some((item) => {
    if (item.kind !== 'file') return false;
    const entry = (
      item as DataTransferItem & {
        webkitGetAsEntry?: () => { isDirectory?: boolean } | null;
      }
    ).webkitGetAsEntry?.();
    return Boolean(entry?.isDirectory);
  });
  return { files, hasFolder };
}

/**
 * A region files can be dropped on: the channel, or the composer on its own.
 *
 * While files are dragged over it the zone says what a drop does and where ("Drop to attach
 * in #general") on a translucent tint, so the messages stay in view. A drop ATTACHES — the file
 * joins the message being written and reaches the channel only on Send — so the words say
 * attach, not share (Q3-14). A drop hands the `File` objects to the registered target, which asks
 * for the native share confirmation (D-DROP) and then uploads through the one transfer path. The zone never reads a file. It is marked `data-drop-zone` so
 * the app-wide handler in `App.tsx` leaves its events to it; everything else still stops the
 * browser from navigating to a dropped file.
 *
 * The target is either the `target` prop or the one a descendant registered with
 * `useCrewDropTarget` (the composer inside the channel), so the layout can make the whole
 * channel a drop zone without knowing how the composer uploads. With no target the zone takes
 * no files and shows nothing.
 */
export function CrewFileDropZone({
  target: ownTarget,
  className,
  children,
}: {
  target?: CrewDropTarget | null;
  className?: string;
  children: ReactNode;
}) {
  const [registered, setRegistered] = useState<CrewDropTarget | null>(null);
  const [dragging, setDragging] = useState(false);
  const depth = useRef(0);
  const register = useCallback((next: CrewDropTarget) => {
    setRegistered(next);
    return () => setRegistered((current) => (current === next ? null : current));
  }, []);
  const registry = useMemo(() => ({ register }), [register]);
  const target = ownTarget ?? registered;

  useEffect(() => {
    if (target) return;
    depth.current = 0;
    setDragging(false);
  }, [target]);

  // A drag can end without a matching `dragleave` here: dropped somewhere else in the window,
  // cancelled, or carried out of the window. Any of those clears the overlay.
  useEffect(() => {
    if (!dragging) return;
    const reset = () => {
      depth.current = 0;
      setDragging(false);
    };
    // Leaving the window is the one `dragleave` with no element to go to and a pointer at or
    // past the edge; judging by the element alone would also clear it between two children.
    const leftWindow = (event: globalThis.DragEvent) => {
      const outside =
        event.clientX <= 0 ||
        event.clientY <= 0 ||
        event.clientX >= window.innerWidth ||
        event.clientY >= window.innerHeight;
      if (event.relatedTarget === null && outside) reset();
    };
    window.addEventListener('drop', reset, true);
    window.addEventListener('dragend', reset, true);
    document.addEventListener('dragleave', leftWindow, true);
    return () => {
      window.removeEventListener('drop', reset, true);
      window.removeEventListener('dragend', reset, true);
      document.removeEventListener('dragleave', leftWindow, true);
    };
  }, [dragging]);

  const onDragEnter = (event: DragEvent<HTMLDivElement>) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth.current += 1;
    if (target) setDragging(true);
  };
  const onDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = target ? 'copy' : 'none';
    // `dragover` repeats while the pointer is here, so a count that drifted re-arms itself.
    if (target && !dragging) {
      depth.current = Math.max(depth.current, 1);
      setDragging(true);
    }
  };
  const onDragLeave = (event: DragEvent<HTMLDivElement>) => {
    if (!carriesFiles(event)) return;
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setDragging(false);
  };
  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth.current = 0;
    setDragging(false);
    if (target) target.onFiles(readDrop(event.dataTransfer));
  };

  return (
    <DropZoneContext.Provider value={registry}>
      <div
        className={cn('crew-drop-zone', className)}
        data-drop-zone="true"
        data-dragging={dragging && target ? 'true' : undefined}
        onDragEnter={onDragEnter}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
      >
        {children}
        {/* A tint over the messages, not a blank: the conversation stays in view under it (Q2-60). */}
        {dragging && target ? (
          <div className="crew-drop-overlay" data-testid="crew-drop-overlay">
            <span className="crew-drop-overlay-label">
              <Upload className="size-5" aria-hidden />
              <span>{filesCopy.dropToAttach(target.channelName)}</span>
            </span>
          </div>
        ) : null}
      </div>
    </DropZoneContext.Provider>
  );
}
