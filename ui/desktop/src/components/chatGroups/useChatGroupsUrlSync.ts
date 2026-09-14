import { useEffect, useRef } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { UserAttachment } from '../../types/message';

export interface UrlOpenRequest {
  sessionId: string;
  initialMessage?: string;
  initialAttachments?: UserAttachment[];
  /** The tab the message that started this chat was typed in, when it was one. */
  originTabId?: string;
  /**
   * The session's name, when the OPENER already knows it.
   *
   * A Recents click is the case that matters: the sidebar is rendering the name
   * on the row you just clicked, so the name is in hand at nav time. Without
   * this the tab was born with the placeholder and only got its real name once
   * BaseChat had fetched the session and fired onSessionUpdate — a visible
   * "New chat" flash on every history click, for a name we already had.
   *
   * Optional, and NOT a replacement for the late rename: a deep link, a fresh
   * chat, or an external nav has no name to pass, and a session renamed later
   * still has to propagate. This just removes the round-trip when it's avoidable.
   */
  title?: string;
  userSetName?: boolean;
}

interface UrlSyncArgs {
  /** The focused session id, or '' when the active group is empty. */
  activeSessionId: string;
  /** Called ONCE per genuinely new ?resumeSessionId= / nav. */
  onOpen: (request: UrlOpenRequest) => void;
}

/**
 * The URL adapter: ONE reader, ONE writer, never mutually recursive.
 *
 * R2 — all three designers flagged an infinite render here as the most likely
 * bug in this feature, because read and write become mutually recursive the
 * moment either fires on the wrong dep. The fixed point is enforced by two
 * mechanisms, and chatGroupsUrlSync.test.tsx asserts it mechanically:
 *
 *   IN  is gated by `lastAppliedParamRef` + `lastAppliedKeyRef` — a given
 *       (param, location.key) pair is consumed exactly once. The key is part of
 *       the gate so that two DELIBERATE navigations to the same session are both
 *       seen, while a re-render with an unchanged location is not. The second one
 *       is harmless: the reducer dedupes by sessionId and merely re-activates the
 *       tab that is already open.
 *
 *   OUT is gated by `replace: true` plus a `!==` check against the CURRENT
 *       param, so a focus mirror that already matches the URL writes nothing.
 *
 *   The echo is closed by `selfWriteRef`. OUT's navigate necessarily produces a
 *       new location.key, which would otherwise re-open IN's gate and feed the
 *       write straight back into a read. selfWriteRef pre-arms the id we are
 *       about to write; IN recognises its own echo, records it as applied, and
 *       returns WITHOUT dispatching. That is the hop that terminates the cycle.
 *
 * The URL encodes ONE session by design: it is a deep-link inbox and a focus
 * mirror, not a description of the window. That is what lets AppSidebar's
 * `currentSessionId` highlight keep working with zero sidebar edits.
 */
export function useChatGroupsUrlSync({ activeSessionId, onOpen }: UrlSyncArgs): void {
  const [searchParams] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();

  const param = searchParams.get('resumeSessionId');
  const locationKey = location.key;

  const lastAppliedParamRef = useRef<string | null>(null);
  const lastAppliedKeyRef = useRef<string | null>(null);
  const selfWriteRef = useRef<string | null>(null);
  // The param IN consumed in THIS commit. openTab is a dispatch, so the reducer
  // state (and with it activeSessionId) only catches up on the next render —
  // OUT must not read the momentary '' beside a just-consumed param as "an
  // empty tab is focused" and clear the very deep link being opened. Lifetime
  // is exactly IN→OUT within one commit: OUT always runs after IN here (hook
  // order) and clears it on every pass.
  const justOpenedRef = useRef<string | null>(null);
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;

  // IN — command, once per param/nav change.
  useEffect(() => {
    if (!param) return;

    // Our own OUT write echoing back. Record it as applied and stop: dispatching
    // here is exactly the read->write->read recursion R2 warns about.
    if (selfWriteRef.current === param) {
      selfWriteRef.current = null;
      lastAppliedParamRef.current = param;
      lastAppliedKeyRef.current = locationKey;
      return;
    }

    if (param === lastAppliedParamRef.current && locationKey === lastAppliedKeyRef.current) {
      return;
    }

    lastAppliedParamRef.current = param;
    lastAppliedKeyRef.current = locationKey;
    justOpenedRef.current = param;

    const state = (location.state ?? {}) as {
      initialMessage?: string;
      initialAttachments?: UserAttachment[];
      originTabId?: string;
      title?: string;
      userSetName?: boolean;
    };

    // A fresh window (a diverge branch opens one) has no react-router
    // location.state, so its canonical title rides the URL instead. Prefer
    // in-app nav state when present, fall back to the URL param — a diverged tab
    // is then born with its real branch name rather than the "New chat"
    // placeholder, matching what Recents shows. The name is backend-resolved and
    // locked, so it is user-set.
    const urlTitle = searchParams.get('resumeSessionTitle') || undefined;
    const title = state.title ?? urlTitle;

    onOpenRef.current({
      sessionId: param,
      initialMessage: state.initialMessage,
      initialAttachments: state.initialAttachments,
      originTabId: typeof state.originTabId === 'string' ? state.originTabId : undefined,
      title,
      userSetName: state.userSetName ?? (urlTitle ? true : undefined),
    });
    // location.state is read, not depended on: a nav is identified by its key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [param, locationKey]);

  // OUT — mirror of focus.
  useEffect(() => {
    const justOpened = justOpenedRef.current;
    justOpenedRef.current = null;

    if (!activeSessionId) {
      // The mirror covers the EMPTY case too (R1-01): when a New chat tab
      // (sidebar button, strip "+", Cmd+T) takes focus, a lingering
      // ?resumeSessionId= points at a chat that is no longer active — the
      // sidebar Recents highlight reads exactly that param, so it kept the
      // previous chat lit. Clear it — unless the param is IN's consumption
      // still in flight this commit (see justOpenedRef).
      if (param && param !== justOpened) {
        navigate('/pair', { replace: true });
      }
      return;
    }
    if (activeSessionId === param) return;

    selfWriteRef.current = activeSessionId;
    navigate(`/pair?resumeSessionId=${activeSessionId}`, { replace: true });
  }, [activeSessionId, param, navigate]);
}
