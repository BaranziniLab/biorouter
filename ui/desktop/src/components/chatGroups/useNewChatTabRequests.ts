import { useEffect, useRef } from 'react';
import { useLocation } from 'react-router-dom';
import type { ChatGroupsAction } from './chatGroupsReducer';

/**
 * The sidebar's New chat, for /pair: open ONE empty tab per navigation.
 *
 * The tab carries sessionId '' until BaseChat's pre-session submit creates a
 * real session; that navigation then ADOPTS this tab in place (see the reducer's
 * adopt branch) rather than orphaning it beside a second one.
 *
 * ARRIVAL. The request that is the very navigation this route MOUNTED on came
 * from another route — Settings, Home, history landing on a new-chat entry — and
 * an arrival focuses a tab already holding an unsent new chat instead of opening
 * a blank one beside it (`OpenTabPayload.resumeUnsent`). Every later New chat was
 * pressed while /pair was on screen, and opens a tab, as it always has.
 *
 * ⚠ "The first new-chat request this mount sees" is NOT the same thing, and was
 * the first version of this rule: arrive on /pair from Recents
 * (?resumeSessionId=…), click a tab holding a draft, press New chat — the press
 * was taken for an arrival, focused the tab already in view and opened nothing;
 * with the draft tab in another pane it jumped there instead. Measured on the
 * desktop and in the production bundle. What makes a request an arrival is the
 * navigation it rode in on, so that is what is compared.
 */
export function useNewChatTabRequests(
  isNewChat: boolean,
  dispatch: ((action: ChatGroupsAction) => void) | undefined
): void {
  const location = useLocation();
  const mountLocationKeyRef = useRef(location.key);
  const handledKeyRef = useRef<string | null>(null);
  useEffect(() => {
    if (!isNewChat || !dispatch) return;
    if (handledKeyRef.current === location.key) return;
    handledKeyRef.current = location.key;
    const arriving = location.key === mountLocationKeyRef.current;
    dispatch({ type: 'openTab', payload: { sessionId: '', resumeUnsent: arriving } });
  }, [isNewChat, location.key, dispatch]);
}
