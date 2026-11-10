import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getSessionUsageLimit,
  putSessionUsageLimit,
  type SessionUsageLimitResponse,
  type SetSessionUsageLimitRequest,
} from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import {
  announceUsageLimitsChanged,
  USAGE_LIMITS_CHANGED_EVENT,
  type UsageLimitsChangedDetail,
} from '../usage/usageLimits';

/**
 * One chat's usage limit as the composer reads it (`GET /sessions/{id}/usage-limit`), and the
 * write the cost card makes (`PUT`). `ChatInput` owns one per composer: it reads the paused state
 * to hold Send and show the composer notice, and hands the same controller to the cost card in
 * the footer, so a change made in the card lifts the notice at once.
 *
 * Read when the chat changes, whenever `refreshKey` changes (`ChatInput` moves it after each turn
 * and when a stored limit notice lands), when the cost card opens (`reload`), and when Settings
 * says a limit changed (`USAGE_LIMITS_CHANGED_EVENT`). Every call carries `userActionHeaders()`,
 * because a private chat's limit is refused without the proof. An answer for a chat the composer
 * no longer shows, or one a newer read has superseded, is dropped.
 */
export interface SessionUsageLimitController {
  /** The chat this controller reads, or `null` when there is none yet (Home, `#/pair`). */
  sessionId: string | null;
  /** The last answer for `sessionId`, or `null` before the first one. */
  state: SessionUsageLimitResponse | null;
  loadFailed: boolean;
  saving: boolean;
  /** Why the last write failed, or `null`. */
  saveError: 'needUser' | 'failed' | null;
  reload: () => void;
  /** Writes this chat's limit; resolves whether it was saved. */
  save: (body: SetSessionUsageLimitRequest) => Promise<boolean>;
  /** Moves each time something asks the cost card to open (the composer notice's action). */
  openCardRequest: number;
  requestOpenCard: () => void;
}

type Held = { sessionId: string; state: SessionUsageLimitResponse };

/** A 403 is the daemon refusing a write it cannot tie to a person (`USAGE_LIMIT_NEED_USER`). */
function saveFailure(status: number | undefined): 'needUser' | 'failed' {
  return status === 403 ? 'needUser' : 'failed';
}

export function useSessionUsageLimit(
  sessionId: string | null | undefined,
  refreshKey: string | number = 0
): SessionUsageLimitController {
  const chatId = sessionId || null;
  const [held, setHeld] = useState<Held | null>(null);
  const [loadFailedFor, setLoadFailedFor] = useState<string | null>(null);
  const [savingFor, setSavingFor] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<SessionUsageLimitController['saveError']>(null);
  const [openCardRequest, setOpenCardRequest] = useState(0);
  const current = useRef(chatId);
  const mounted = useRef(true);
  const seq = useRef(0);
  const self = useRef({});

  useEffect(() => {
    current.current = chatId;
    setSaveError(null);
  }, [chatId]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const load = useCallback(async (forSession: string) => {
    const mine = ++seq.current;
    const wanted = () => mounted.current && current.current === forSession && seq.current === mine;
    try {
      const result = await getSessionUsageLimit({
        path: { session_id: forSession },
        headers: await userActionHeaders(),
        throwOnError: false,
      });
      if (!result.data) throw new Error('usage limit');
      if (!wanted()) return;
      setHeld({ sessionId: forSession, state: result.data });
      setLoadFailedFor(null);
    } catch {
      if (!wanted()) return;
      setLoadFailedFor(forSession);
    }
  }, []);

  const reload = useCallback(() => {
    if (current.current) void load(current.current);
  }, [load]);

  useEffect(() => {
    if (chatId) void load(chatId);
  }, [chatId, refreshKey, load]);

  useEffect(() => {
    const onChanged = (event: Event) => {
      const detail = (event as CustomEvent<UsageLimitsChangedDetail | undefined>).detail;
      // This controller's own write already holds the answer.
      if (detail?.source === self.current) return;
      reload();
    };
    window.addEventListener(USAGE_LIMITS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(USAGE_LIMITS_CHANGED_EVENT, onChanged);
  }, [reload]);

  const save = useCallback(async (body: SetSessionUsageLimitRequest) => {
    const forSession = current.current;
    if (!forSession) return false;
    setSavingFor(forSession);
    setSaveError(null);
    // A write supersedes any read in flight: its answer is the newer state.
    const mine = ++seq.current;
    try {
      const result = await putSessionUsageLimit({
        path: { session_id: forSession },
        headers: await userActionHeaders(),
        body,
        throwOnError: false,
      });
      if (!result.data) {
        if (mounted.current && current.current === forSession) {
          setSaveError(saveFailure(result.response?.status));
        }
        return false;
      }
      if (mounted.current && current.current === forSession && seq.current === mine) {
        setHeld({ sessionId: forSession, state: result.data });
        setLoadFailedFor(null);
      }
      // Another pane showing the same chat reads it again.
      announceUsageLimitsChanged(forSession, self.current);
      return true;
    } catch {
      if (mounted.current && current.current === forSession) setSaveError('failed');
      return false;
    } finally {
      if (mounted.current) setSavingFor((now) => (now === forSession ? null : now));
    }
  }, []);

  const requestOpenCard = useCallback(() => setOpenCardRequest((n) => n + 1), []);

  return {
    sessionId: chatId,
    state: chatId && held?.sessionId === chatId ? held.state : null,
    loadFailed: chatId !== null && loadFailedFor === chatId,
    saving: chatId !== null && savingFor === chatId,
    saveError,
    reload,
    save,
    openCardRequest,
    requestOpenCard,
  };
}
