import { isBrowserSurface } from './surface';

/**
 * When a session folder grant ends, in ONE sentence: it is read inside an InfoTip (the composer
 * folder card's "Other approved folders", the transcript's folder approval), where help is kept to
 * two sentences, and both surfaces add a sentence of their own beside it.
 */
export function folderAccessLifetime(): string {
  const managedDesktop =
    !isBrowserSurface() && window.appConfig?.get('BIOROUTER_LOCAL_BACKEND') === true;
  return managedDesktop
    ? 'Session grants expire when revoked, this chat is deleted, or the desktop app quits or restarts.'
    : 'Session grants expire when revoked, this chat is deleted, or the backend restarts; closing or restarting this interface does not revoke them.';
}
