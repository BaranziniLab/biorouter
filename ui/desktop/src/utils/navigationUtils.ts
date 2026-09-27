import { NavigateFunction } from 'react-router-dom';
import { Workflow } from '../api/types.gen';
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
};

export const navigateWithViewTransition = (
  navigate: NavigateFunction,
  to: string,
  state?: unknown,
  options: { replace?: boolean } = {}
) => {
  navigate(to, { replace: options.replace, state });
};

export const createNavigationHandler = (navigate: NavigateFunction) => {
  return (view: View, options?: ViewOptions) => {
    switch (view) {
      case 'crew':
        navigateWithViewTransition(
          navigate,
          options?.resumeSessionId
            ? `/crew?sessionId=${encodeURIComponent(options.resumeSessionId)}`
            : '/crew',
          options
        );
        break;
      case 'chat':
        navigateWithViewTransition(navigate, '/', options);
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
        navigateWithViewTransition(
          navigate,
          options?.resumeSessionId
            ? `/pair?resumeSessionId=${encodeURIComponent(options.resumeSessionId)}`
            : '/pair',
          options
        );
        break;
      case 'settings':
        navigateWithViewTransition(navigate, '/settings', options);
        break;
      case 'sessions':
        navigateWithViewTransition(navigate, '/sessions', options);
        break;
      case 'schedules':
        navigateWithViewTransition(navigate, '/schedules', options);
        break;
      case 'workflows':
        navigateWithViewTransition(navigate, '/workflows', options);
        break;
      case 'skills':
        navigateWithViewTransition(navigate, '/skills', options);
        break;
      case 'permission':
        navigateWithViewTransition(navigate, '/permission', options);
        break;
      case 'ConfigureProviders':
        navigateWithViewTransition(navigate, '/configure-providers', options);
        break;
      case 'sharedSession':
        navigateWithViewTransition(navigate, '/shared-session', options);
        break;

      case 'welcome':
        navigateWithViewTransition(navigate, '/welcome', options);
        break;
      case 'extensions':
        navigateWithViewTransition(navigate, '/extensions', options);
        break;
      default:
        navigateWithViewTransition(navigate, '/', options);
    }
  };
};
