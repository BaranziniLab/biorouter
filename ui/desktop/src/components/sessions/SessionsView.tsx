import React, { useCallback, useEffect } from 'react';
import SessionListView from './SessionListView';
import { useLocation } from 'react-router-dom';
import { useNavigation } from '../../hooks/useNavigation';

/**
 * Chat history: the list, and one thing you can do with a row — open it.
 *
 * ⚠ **This view never renders `SessionHistoryView`, and it never could.** It
 * used to carry a second branch that showed the read-only transcript in place,
 * gated on a `showSessionHistory` flag — and that flag was set in exactly one
 * function, `loadSessionDetails`, which was called from exactly one place,
 * `handleRetryLoadSession`, which returned early unless `selectedSession` was
 * set, which only `loadSessionDetails` ever set. A closed cycle with no way in:
 * the fetch, its loading and error states, the retry, the back button and the
 * placeholder session object were all unreachable from every path through the
 * app. The effect below, which looks like the way in, calls
 * `handleSelectSession` — which navigates to `/pair`.
 *
 * Resuming the chat is the SHIPPED behaviour and the intended one (a row in
 * Chat history opens the conversation, it does not show a frozen copy of it),
 * so the branch is gone rather than being wired up. `SessionHistoryView` itself
 * stays: it is the read-only transcript the schedule run detail and the shared
 * session both mount, and both reach it directly.
 */
const SessionsView: React.FC = () => {
  const location = useLocation();
  const setView = useNavigation();

  const handleSelectSession = useCallback(
    async (sessionId: string) => {
      setView('pair', {
        disableAnimation: true,
        resumeSessionId: sessionId,
      });
    },
    [setView]
  );

  // A session id handed over in the location state (from SessionsInsights on
  // Home) opens that chat directly rather than leaving the user on the list.
  useEffect(() => {
    const state = location.state as { selectedSessionId?: string } | null;
    if (state?.selectedSessionId) {
      handleSelectSession(state.selectedSessionId);
      // Clear the state to prevent reloading on navigation
      window.history.replaceState({}, document.title);
    }
  }, [location.state, handleSelectSession]);

  // `selectedSessionId` is left unset rather than passed as `null`. It scrolls a
  // named row into view "when returning from session history view" — the
  // journey that no longer exists — and this view was already passing a value
  // that could only ever be null. The prop stays on `SessionListView` because
  // it is optional and describes something a caller may legitimately want; what
  // is removed here is the pretence that this caller supplies it.
  return <SessionListView onSelectSession={handleSelectSession} />;
};

export default SessionsView;
