import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { getSession } from '../../api';
import type { DictionaryAuthority } from '../../api';
import { toastError } from '../../toasts';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import { dictationMessage } from '../dictation/copy';
import {
  ensureDictationStatus,
  getDictationStatusSnapshot,
  subscribeDictationStatus,
} from '../dictation/dictationStore';
import type { DictionaryAddTermPayload } from './contextMenu';
import TermDialog, { type TermDialogMode } from './TermDialog';
import { originAt, type OriginDeps, type TermOrigin } from './termOrigin';
import { loadDictionarySettings } from './useDictionary';

/**
 * Whether **Only in private chats** starts ticked for an add from `origin`: for
 * a private chat, and for a transcript whose chat could not be resolved (fail
 * toward private); a chat whose tier cannot be read counts as private too.
 * Outside any transcript it starts unticked (O-6).
 */
export async function startsPrivate(
  origin: TermOrigin,
  readTier: (sessionId: string) => Promise<string | null> = readChatTier
): Promise<boolean> {
  if (origin.kind === 'none') return false;
  if (origin.kind === 'unresolved') return true;
  const tier = await readTier(origin.sessionId);
  return tier !== 'public';
}

/** The chat's stored tier (`privacy_tier`), with the proof; `null` when it cannot be read. */
export async function readChatTier(sessionId: string): Promise<string | null> {
  try {
    const response = await getSession({
      path: { session_id: sessionId },
      query: { metadata_only: true },
      headers: await userActionHeaders(),
    });
    return response.data?.privacy_tier ?? null;
  } catch {
    return null;
  }
}

/** What the door needs to open a dialog, before the dialog itself. */
export async function prepareAddFromMenu(
  payload: DictionaryAddTermPayload,
  deps: OriginDeps & { readTier?: (sessionId: string) => Promise<string | null> } = {}
): Promise<Extract<TermDialogMode, { kind: 'add' }>> {
  // The point is read at once: the layout can change while the tier is read.
  const origin = originAt(payload.x, payload.y, deps);
  const startPrivate = await startsPrivate(origin, deps.readTier);
  return { kind: 'add', text: payload.text, origin, startPrivate };
}

/** What the window's dictation status store says of the dictionary: not read yet, or the policy's answer. */
type DictionaryOffer = 'unknown' | 'offered' | 'withheld';

function dictionaryOffer(): DictionaryOffer {
  const status = getDictationStatusSnapshot().status;
  if (!status) return 'unknown';
  return status.policy.dictionary.allowed === false ? 'withheld' : 'offered';
}

const unknownOffer = (): DictionaryOffer => 'unknown';

/**
 * The context menu's door (DD-68): on `dictionary:add-term` it resolves the
 * pane under the menu's point, decides whether **Only in private chats** starts
 * ticked, and opens `TermDialog` prefilled with the word. Mounted once per
 * window by `App.tsx`; it renders nothing until the menu is used, and nothing
 * on a `serve` browser, which has no Electron menu (and whose dictation
 * controls wait for PR-21).
 *
 * Managed `dictionary: false` withholds the door as it hides the sidebar row:
 * once the window's status store has read the policy, the door tells main
 * whether to offer the item (`dictionary:offered`), and a click that arrives
 * under the policy anyway (a menu opened before the store read it) shows the
 * policy's sentence instead of a dialog whose add could only be refused. The
 * door reads the store but loads it only on a click, so mounting sends nothing.
 */
export default function DictionaryTermDoor() {
  const [mode, setMode] = useState<Extract<TermDialogMode, { kind: 'add' }> | null>(null);
  const [authority, setAuthority] = useState<DictionaryAuthority | null>(null);
  // A second word picked while the dialog is open replaces it with a fresh one.
  const [opened, setOpened] = useState(0);
  const offer = useSyncExternalStore(subscribeDictationStatus, dictionaryOffer, unknownOffer);

  const open = useCallback(async (payload: DictionaryAddTermPayload) => {
    // Each read fails toward the dialog: no authority leaves the daemon to refuse what it
    // must, and no status leaves the policy to the daemon too.
    const [next, settings, status] = await Promise.all([
      prepareAddFromMenu(payload),
      loadDictionarySettings().catch(() => null),
      ensureDictationStatus().catch(() => null),
    ]);
    if (status?.policy.dictionary.allowed === false) {
      toastError({
        title: payload.text,
        msg: status.policy.dictionary.reason ?? dictationMessage('MSG-POLICY'),
      });
      return;
    }
    setAuthority(settings?.authority ?? null);
    setMode(next);
    setOpened((count) => count + 1);
  }, []);

  useEffect(() => {
    if (isBrowserSurface()) return;
    const subscribe = window.electron?.onDictionaryAddTerm;
    if (typeof subscribe !== 'function') return;
    return subscribe((payload) => void open(payload));
  }, [open]);

  // Until the store has read the policy the window says nothing, so main keeps what it last
  // heard (offered, for a new window), as the sidebar shows its row until then.
  useEffect(() => {
    if (isBrowserSurface() || offer === 'unknown') return;
    window.electron?.setDictionaryOffered?.(offer === 'offered');
  }, [offer]);

  if (!mode) return null;
  return (
    <TermDialog key={opened} mode={mode} authority={authority} onClose={() => setMode(null)} />
  );
}
