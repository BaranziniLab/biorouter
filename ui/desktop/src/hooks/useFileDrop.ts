import { useCallback, useState, useRef, useEffect } from 'react';

export interface DroppedFile {
  id: string;
  path: string;
  sourcePath?: string;
  stagedPath?: string;
  name: string;
  type: string;
  isImage: boolean;
  canUploadAsImage?: boolean;
  dataUrl?: string; // For image previews
  isLoading?: boolean;
  error?: string;
}

const UPLOADABLE_IMAGE_TYPES = new Set([
  'image/png',
  'image/jpeg',
  'image/jpg',
  'image/gif',
  'image/webp',
]);
const MAX_STAGED_IMAGE_BYTES = 3 * 1024 * 1024;

async function validateImageDataUrl(dataUrl: string): Promise<void> {
  if (
    typeof Image === 'undefined' ||
    typeof HTMLImageElement === 'undefined' ||
    typeof HTMLImageElement.prototype.decode !== 'function'
  ) {
    return;
  }

  const img = new Image();
  img.src = dataUrl;
  await img.decode();
}

function fileNameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

function decodeFileUri(uri: string): string | null {
  const trimmed = uri.trim();
  if (!trimmed || trimmed.startsWith('#')) return null;
  try {
    const url = new URL(trimmed);
    if (url.protocol !== 'file:') return null;
    return decodeURIComponent(url.pathname);
  } catch {
    return null;
  }
}

function getDroppedPathCandidates(dataTransfer: DataTransfer): string[] {
  const candidates = new Set<string>();
  const uriList = dataTransfer.getData('text/uri-list');
  for (const line of uriList.split(/\r?\n/)) {
    const path = decodeFileUri(line);
    if (path) candidates.add(path);
  }

  const plainText = dataTransfer.getData('text/plain').trim();
  if (plainText.startsWith('file://')) {
    const path = decodeFileUri(plainText);
    if (path) candidates.add(path);
  }

  return [...candidates];
}

/**
 * @param initialFiles what the drop tray starts with — a composer rebuilding a
 *   new chat's unsent message (`utils/composerDrafts.ts`) seeds it here, in the
 *   first render, so nothing mounted can observe an empty tray first. Read once.
 */
export const useFileDrop = (initialFiles?: () => DroppedFile[]) => {
  const [droppedFiles, setDroppedFiles] = useState<DroppedFile[]>(() => initialFiles?.() ?? []);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const activeReadersRef = useRef<Set<FileReader>>(new Set());
  const dragDepthRef = useRef(0);

  // Cleanup effect to prevent memory leaks
  useEffect(() => {
    return () => {
      // Abort any active FileReaders on unmount
      // eslint-disable-next-line react-hooks/exhaustive-deps
      const readers = activeReadersRef.current;
      readers.forEach((reader) => {
        try {
          reader.abort();
        } catch {
          // Reader might already be done, ignore errors
        }
      });
      readers.clear();
    };
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    dragDepthRef.current = 0;
    setIsDraggingOver(false);
    const files = e.dataTransfer.files;
    const seenPaths = new Set<string>();
    const droppedFileObjects: DroppedFile[] = [];

    if (files.length > 0) {
      for (let i = 0; i < files.length; i++) {
        const file = files[i];

        let droppedFile: DroppedFile;

        try {
          const sourcePath = window.electron.getPathForFile(file);
          const canUploadAsImage =
            UPLOADABLE_IMAGE_TYPES.has(file.type.toLowerCase()) &&
            file.size <= MAX_STAGED_IMAGE_BYTES;

          // ⚠ NEVER fall back to `file.name` when there is no source path.
          //
          // This used to read `sourcePath || file.name`, which looks like a
          // harmless default and is not. A bare name is resolved against the
          // working directory of whatever reads it -- and in browser mode that
          // is the SERVER, a different machine from the one the file was
          // dragged off. Dropping `results.csv` could therefore hand the agent
          // a completely different `results.csv` that happened to exist there,
          // with nothing anywhere reporting a problem.
          //
          // Images are exempt because they never need a path: they are read
          // here as data URLs and staged by `saveDataUrlToTemp`, which is why
          // dropping an image still works with no filesystem access at all.
          if (!sourcePath && !canUploadAsImage) {
            droppedFileObjects.push({
              id: `dropped-nopath-${Date.now()}-${i}`,
              path: '',
              // Explicitly absent, matching the shape of the normal branch, so
              // nothing downstream can read a stale or invented source.
              sourcePath: undefined,
              name: file.name,
              type: file.type,
              isImage: file.type.startsWith('image/'),
              canUploadAsImage: false,
              isLoading: false,
              error:
                'This file cannot be read from a browser tab. Biorouter runs on ' +
                'another machine here, so it has no way to reach the file you ' +
                'dropped. Copy it onto that machine and reference it by path, ' +
                'or paste its contents into the message.',
            });
            continue;
          }

          droppedFile = {
            id: `dropped-${Date.now()}-${i}`,
            path: sourcePath,
            sourcePath: sourcePath || undefined,
            name: file.name,
            type: file.type,
            isImage: file.type.startsWith('image/'),
            canUploadAsImage,
            isLoading: canUploadAsImage,
          };
          if (sourcePath) {
            seenPaths.add(sourcePath);
          }
        } catch (error) {
          console.error('Error processing file:', file.name, error);
          // Create an error file object
          droppedFile = {
            id: `dropped-error-${Date.now()}-${i}`,
            path: '',
            name: file.name,
            type: file.type,
            isImage: false,
            canUploadAsImage: false,
            isLoading: false,
            error: `Failed to get file path: ${error instanceof Error ? error.message : 'Unknown error'}`,
          };
        }

        droppedFileObjects.push(droppedFile);

        // For images, generate a preview AND persist a temp copy so the send
        // path can read the bytes through the same temp-dir IPC the paste flow
        // uses. The OS-supplied source path is rejected by the IPC's path
        // validation (and may be absent altogether for synthetic File objects),
        // so we cannot rely on it for the model-bound base64 read.
        if (droppedFile.canUploadAsImage && !droppedFile.error) {
          const reader = new FileReader();
          activeReadersRef.current.add(reader);

          reader.onload = async (event) => {
            const dataUrl = event.target?.result as string;
            try {
              await validateImageDataUrl(dataUrl);
            } catch {
              setDroppedFiles((prev) =>
                prev.map((f) =>
                  f.id === droppedFile.id
                    ? {
                        ...f,
                        dataUrl: undefined,
                        canUploadAsImage: false,
                        stagedPath: undefined,
                        isLoading: false,
                        error: undefined,
                      }
                    : f
                )
              );
              activeReadersRef.current.delete(reader);
              return;
            }

            try {
              const saved = await window.electron.saveDataUrlToTemp(dataUrl, droppedFile.id);
              if (saved.error || !saved.filePath) {
                throw new Error(saved.error ?? 'saveDataUrlToTemp returned no path');
              }
              setDroppedFiles((prev) =>
                prev.map((f) =>
                  f.id === droppedFile.id
                    ? {
                        ...f,
                        dataUrl,
                        stagedPath: saved.filePath as string,
                        isLoading: false,
                      }
                    : f
                )
              );
            } catch (err) {
              setDroppedFiles((prev) =>
                prev.map((f) =>
                  f.id === droppedFile.id
                    ? {
                        ...f,
                        dataUrl,
                        isLoading: false,
                        error: `Failed to stage image: ${err instanceof Error ? err.message : String(err)}`,
                      }
                    : f
                )
              );
            } finally {
              activeReadersRef.current.delete(reader);
            }
          };

          reader.onerror = () => {
            console.error('Failed to generate preview for:', file.name);
            setDroppedFiles((prev) =>
              prev.map((f) =>
                f.id === droppedFile.id
                  ? { ...f, error: 'Failed to load image preview', isLoading: false }
                  : f
              )
            );
            activeReadersRef.current.delete(reader);
          };

          reader.onabort = () => {
            activeReadersRef.current.delete(reader);
          };

          reader.readAsDataURL(file);
        }
      }
    }

    for (const path of getDroppedPathCandidates(e.dataTransfer)) {
      if (seenPaths.has(path)) continue;
      droppedFileObjects.push({
        id: `dropped-path-${Date.now()}-${droppedFileObjects.length}`,
        path,
        sourcePath: path,
        name: fileNameFromPath(path),
        type: '',
        isImage: false,
        canUploadAsImage: false,
        isLoading: false,
      });
    }

    if (droppedFileObjects.length > 0) {
      setDroppedFiles((prev) => [...prev, ...droppedFileObjects]);
    }
  }, []);

  const handleDragEnter = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    dragDepthRef.current += 1;
    setIsDraggingOver(true);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = 'copy';
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) {
      setIsDraggingOver(false);
    }
  }, []);

  return {
    droppedFiles,
    setDroppedFiles,
    isDraggingOver,
    handleDrop,
    handleDragEnter,
    handleDragOver,
    handleDragLeave,
  };
};
