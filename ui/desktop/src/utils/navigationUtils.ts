import { useLayoutEffect, useRef } from 'react';
import { NavigateFunction } from 'react-router-dom';
import { DUR, EASE_OUT, isWindowResizing, prefersReducedMotion } from '../styles/motion';
import { Workflow, type SessionClassification } from '../api/types.gen';
import type { UserAttachment } from '../types/message';

export type View =
  | 'crew'
  | 'welcome'
  | 'chat'
  | 'pair'
  | 'settings'
  | 'extensions'
  | 'moreModels'
  | 'configureProviders'
  | 'configPage'
  | 'ConfigureProviders'
  | 'settingsV2'
  | 'sessions'
  | 'schedules'
  | 'sharedSession'
  | 'loading'
  | 'workflows'
  | 'skills'
  | 'permission';

export type ViewOptions = {
  showEnvVars?: boolean;
  deepLinkConfig?: unknown;
  sessionDetails?: unknown;
  error?: string;
  baseUrl?: string;
  workflow?: Workflow;
  parentView?: View;
  parentViewOptions?: ViewOptions;
  disableAnimation?: boolean;
  initialMessage?: string;
  initialAttachments?: UserAttachment[];
  /** The chat tab a pre-session submit was typed in (`OpenTabPayload.originTabId`). */
  originTabId?: string;
  shareToken?: string;
  resumeSessionId?: string;
  pendingScheduleDeepLink?: string;
  /**
   * The route to go back to when a detour ends: the provider catalog opened
   * from a chat's model picker returns there (W2-PRV-5).
   */
  returnTo?: string;
  /** The tier of the chat `resumeSessionId` names, for a picker opened on its behalf. */
  privacyTier?: SessionClassification;
  /**
   * The tab of a chat not sent yet, for a picker opened on its behalf: the
   * provider catalog holds the model chosen after a setup for that chat
   * (`pendingChatModel.ts`) instead of making it every new chat's (T3-SH-2).
   */
  heldChatTabId?: string;
};

/**
 * Go to a route. It is a plain `navigate`: the change is synchronous, with no
 * View Transition and no document snapshot (spec 3.16, "no exit, no slide and
 * no View Transitions"), and the only motion is the main column's one fade in
 * {@link useRouteFade}.
 *
 * It was called `navigateWithViewTransition`, a name that promised a motion the
 * function never made.
 */
export const navigateToRoute = (
  navigate: NavigateFunction,
  to: string,
  state?: unknown,
  options: { replace?: boolean } = {}
) => {
  navigate(to, { replace: options.replace, state });
};

/**
 * @deprecated The old name of {@link navigateToRoute}, kept only until its last
 * importer (`components/BaseChat.tsx`, WS-SUMMARY) moves; then it is deleted.
 */
export const navigateWithViewTransition = navigateToRoute;

/** The app shell's route surface: the `<main>` that holds every routed view (`AppLayout`). */
export const ROUTE_SURFACE_SELECTOR = 'main.biorouter-route-surface';

/**
 * Routes (spec 3.16): when the path changes, the main column fades in once,
 * opacity only, over `--dur-fast` on `--ease-out`. No exit, no slide, no View
 * Transition, and nothing on the first mount (a reload, a window opening), on a
 * change of query string alone, under reduced motion or while the window is
 * being resized.
 *
 * A layout effect, so the new route's first frame is already at opacity 0 and
 * never flashes in at full ink first. WAAPI rather than a keyed wrapper: the
 * surface is not remounted and gains no box, so no routed view's layout moves.
 */
export function useRouteFade(pathname: string): void {
  const shown = useRef(pathname);
  useLayoutEffect(() => {
    if (shown.current === pathname) return;
    shown.current = pathname;
    if (prefersReducedMotion() || isWindowResizing()) return;
    if (typeof document === 'undefined') return;
    const surface = document.querySelector<HTMLElement>(ROUTE_SURFACE_SELECTOR);
    if (!surface || typeof surface.animate !== 'function') return;
    surface.animate([{ opacity: 0 }, { opacity: 1 }], {
      duration: DUR.fast,
      easing: EASE_OUT,
    });
  }, [pathname]);
}

export const createNavigationHandler = (navigate: NavigateFunction) => {
  return (view: View, options?: ViewOptions) => {
    switch (view) {
      case 'crew':
        navigateToRoute(
          navigate,
          options?.resumeSessionId
            ? `/crew?sessionId=${encodeURIComponent(options.resumeSessionId)}`
            : '/crew',
          options
        );
        break;
      case 'chat':
        navigateToRoute(navigate, '/', options);
        break;
      case 'pair':
        // The session id MUST ride in the query string, not only in location.state.
        //
        // /pair's deep-link inbox is useChatGroupsUrlSync, and its IN effect is
        // gated on `searchParams.get('resumeSessionId')` — it returns early when
        // that param is absent, BEFORE it ever reads location.state. So a
        // state-only navigation reached the route with the message in hand and
        // dropped it on the floor: no openTab, no tab cargo, no submit. That is
        // the Home-composer "my message vanished" bug (Hub.tsx passes
        // resumeSessionId + initialMessage here), and it hit resumeSession(),
        // startNewSession(), WorkflowsView and SessionsView identically.
        //
        // App.tsx's rescue effect could not cover it: it only fires when there is
        // NO session id, and these callers arrive with one already created.
        //
        // The options object still travels as location.state, which is where
        // initialMessage / initialAttachments / title are read from once the
        // param has opened the gate.
        navigateToRoute(
          navigate,
          options?.resumeSessionId
            ? `/pair?resumeSessionId=${encodeURIComponent(options.resumeSessionId)}`
            : '/pair',
          options
        );
        break;
      case 'settings':
        navigateToRoute(navigate, '/settings', options);
        break;
      case 'sessions':
        navigateToRoute(navigate, '/sessions', options);
        break;
      case 'schedules':
        navigateToRoute(navigate, '/schedules', options);
        break;
      case 'workflows':
        navigateToRoute(navigate, '/workflows', options);
        break;
      case 'skills':
        navigateToRoute(navigate, '/skills', options);
        break;
      case 'permission':
        navigateToRoute(navigate, '/permission', options);
        break;
      case 'ConfigureProviders':
        navigateToRoute(navigate, '/configure-providers', options);
        break;
      case 'sharedSession':
        navigateToRoute(navigate, '/shared-session', options);
        break;

      case 'welcome':
        navigateToRoute(navigate, '/welcome', options);
        break;
      case 'extensions':
        navigateToRoute(navigate, '/extensions', options);
        break;
      default:
        navigateToRoute(navigate, '/', options);
    }
  };
};
