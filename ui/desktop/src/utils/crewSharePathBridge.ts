/**
 * The preload half of D-DROP: sharing a file a person dropped or pasted into Crew.
 *
 * The renderer hands over the `File` it received from a real drop or paste event, never a path.
 * This module turns that `File` into a path with Electron's `webUtils.getPathForFile`, which
 * answers `''` for anything that is not a file on disk (a synthetic `new File(...)`, pasted
 * image data, a browser surface), and asks the main process to confirm the share. The main
 * process shows a native dialog naming the file, its size, its full path and the destination,
 * and only a click on Share there produces a daemon file capability (`crew:share-dropped-file`
 * in `main.ts`, the rules in `crewSharePath.ts`).
 *
 * Kept free of Node imports: the preload bundle may run sandboxed, and everything here is a
 * type, a string or a function over two injected Electron objects, so it is testable without
 * Electron.
 */

/** The IPC channel `main.ts` answers. The preload and the handler must agree on it. */
export const CREW_SHARE_DROPPED_FILE_CHANNEL = 'crew:share-dropped-file';

/**
 * Where the dropped file goes, as the renderer knows it.
 *
 * `connectionId` and `channelId` bind the daemon capability, exactly as they do for the Attach
 * picker (`crewSelectTransferFile`). `channelName` and `workspaceName` are display text for the
 * native dialog only; the main process strips hidden characters from them and caps their length.
 */
export interface CrewShareDestination {
  expectedMode?: 'private' | 'public';
  connectionId: string;
  channelId: string;
  /** The channel's name, with or without a leading `#`. */
  channelName: string;
  workspaceName: string;
}

/**
 * The main process's answer.
 *
 * - `shared`: the person chose Share. `capability_id` is the same opaque daemon file capability
 *   the Attach picker returns; start the upload with it exactly as `beginTransfer` does
 *   (`POST /crew/transfers` with `file_capability`). `name` and `size` are what the daemon
 *   opened, which the main process checked against what the dialog showed.
 * - `cancelled`: the person chose Cancel or closed the dialog, or the window went away. Show
 *   nothing.
 * - `refused`: the file cannot be shared. `message` is one plain sentence for the person.
 */
export type CrewShareDroppedFileResult =
  | { outcome: 'shared'; capability_id: string; name: string; size: number }
  | { outcome: 'cancelled' }
  | { outcome: 'refused'; message: string };

export type CrewShareDroppedFile = (
  file: File,
  destination: CrewShareDestination
) => Promise<CrewShareDroppedFileResult>;

interface PathForFile {
  getPathForFile(file: File): string;
}
interface Invoker {
  invoke(channel: string, ...args: unknown[]): Promise<unknown>;
}

/**
 * Builds `window.electron.crewShareDroppedFile`.
 *
 * Only the named destination fields cross to the main process, whatever else the caller's
 * object holds, and the path is resolved here from the `File`, never read from the caller.
 */
export function createCrewShareDroppedFile(
  webUtils: PathForFile,
  ipcRenderer: Invoker
): CrewShareDroppedFile {
  return (file, destination) => {
    let droppedPath = '';
    try {
      const resolved = webUtils.getPathForFile(file);
      if (typeof resolved === 'string') droppedPath = resolved;
    } catch {
      // Not a File at all. The main process refuses an empty path with a plain sentence.
      droppedPath = '';
    }
    return ipcRenderer.invoke(CREW_SHARE_DROPPED_FILE_CHANNEL, {
      path: droppedPath,
      connectionId: destination?.connectionId,
      channelId: destination?.channelId,
      channelName: destination?.channelName,
      workspaceName: destination?.workspaceName,
      expectedMode: destination?.expectedMode,
    }) as Promise<CrewShareDroppedFileResult>;
  };
}
