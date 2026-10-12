import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { getSession } from '../../api';
import { getCachedSessionList, subscribeSessionList } from '../../utils/sessionListCache';
import { userActionHeaders } from '../../utils/userAction';
import { getInitialWorkingDir } from '../../utils/workingDir';
import { readComposerHandOff, useComposerHandOff } from '../composer/composerHandOff';

const noSubscription = () => () => {};
const noRows = () => null;

/** A folder this composer knows, and the chat it is the folder of. */
interface KnownFolder {
  chat: string | null;
  dir: string;
}

const FOLDER_HAND_OFF = 'working-dir';

/**
 * The folder the composer's footer names (spec 3.7), as a fact about THIS chat.
 *
 * ⚠ **A chat that exists never shows the app's default folder** (vision V2-R7-3,
 * principle 10). The footer used to read `sessionWorkingDir ??
 * getInitialWorkingDir()`, so every chat opened on "home" and corrected itself
 * when its row arrived: 137 to 360 ms on most chats, 3.2 s on a long one. A
 * folder shown and then withdrawn is the class V2-R5-5 fixed for the summary
 * rail. Where the folder comes from, in order:
 *
 * 1. the chat's own row, `GET /sessions/{id}?metadata_only=true` with the
 *    person's proof (a private chat refuses a caller without it, issue #56). It
 *    goes out with the loading chat's read of the same row, and
 *    `utils/sessionReadCoalescing` sends one request for both;
 * 2. until then, the session listing's row (`utils/sessionListCache`, kept warm
 *    by the sidebar and History), which most opens hold in the first frame;
 * 3. otherwise nothing: `null`, and the footer leaves the folder out until it
 *    is known.
 *
 * Before a chat exists (Home, a new tab) the folder IS the default, or the one
 * picked here, which is what the chat will be created in (`createSession` gets
 * `pendingWorkingDir ?? getInitialWorkingDir()`). So when this composer's chat
 * comes into being (the first message of a new tab, the tab keeping its mount),
 * the folder it showed stays until the new row answers: it is the folder the
 * chat was created in, and hiding it for one round trip would blink the footer
 * at the moment of Send.
 *
 * ⚠ V2-R8-3: that first message also REMOUNTS the composer (it moves from the
 * empty chat's centre to the bar under the transcript). What this composer
 * knows is handed to the new mount under `handOffKey`
 * (`composer/composerHandOff.ts`); kept in state alone, a folder picked before
 * the send fell back to the default until the new chat's row answered.
 *
 * @param handOffKey the composer's lasting identity (`ChatInput`'s hand-off key).
 * @returns `workingDir`, the folder to name, or `null` for a chat whose folder
 *   is not known yet; `setWorkingDir`, for a folder the person just picked.
 */
export function useSessionWorkingDir(
  sessionId: string | null | undefined,
  handOffKey?: string
): {
  workingDir: string | null;
  setWorkingDir: (dir: string) => void;
} {
  const chat = sessionId || null;
  // What this composer knows, keyed by the chat it is about, so nothing read
  // for one chat is ever shown under another. A replaced composer's knowledge
  // starts this one, with the chat it was about.
  const [handedOver] = useState(() =>
    readComposerHandOff<KnownFolder>(handOffKey, FOLDER_HAND_OFF)
  );
  const [known, setKnown] = useState<KnownFolder | null>(handedOver);
  const [knownFor, setKnownFor] = useState(handedOver ? handedOver.chat : chat);
  useComposerHandOff(handOffKey, FOLDER_HAND_OFF, known);
  if (knownFor !== chat) {
    setKnownFor(chat);
    setKnown(
      knownFor === null && chat !== null
        ? { chat, dir: known?.chat === null ? known.dir : getInitialWorkingDir() }
        : null
    );
  }
  const knownDir = known !== null && known.chat === chat ? known.dir : null;

  // Subscribed only while the folder is unknown, so a rename elsewhere does not
  // re-render the composer.
  const wanted = chat !== null && knownDir === null;
  const subscribe = useCallback(
    (onChange: () => void) => (wanted ? subscribeSessionList(onChange) : noSubscription()),
    [wanted]
  );
  const rows = useSyncExternalStore(
    subscribe,
    wanted ? getCachedSessionList : noRows,
    wanted ? getCachedSessionList : noRows
  );
  const listedDir = wanted ? rows?.find((row) => row.id === chat)?.working_dir || null : null;

  useEffect(() => {
    if (!chat) return;
    let current = true;
    void (async () => {
      try {
        const response = await getSession({
          path: { session_id: chat },
          // The row, not the transcript: `working_dir` is all this reads.
          query: { metadata_only: true },
          // Issue #56 Task 58: reading a private chat needs the proof-of-user.
          headers: await userActionHeaders(),
        });
        const dir = response.data?.working_dir;
        if (current && dir) setKnown({ chat, dir });
      } catch (error) {
        console.error('[ChatInput] Failed to fetch session working dir:', error);
      }
    })();
    return () => {
      current = false;
    };
  }, [chat]);

  const setWorkingDir = useCallback((dir: string) => setKnown({ chat, dir }), [chat]);

  return {
    workingDir: chat === null ? (knownDir ?? getInitialWorkingDir()) : (knownDir ?? listedDir),
    setWorkingDir,
  };
}
