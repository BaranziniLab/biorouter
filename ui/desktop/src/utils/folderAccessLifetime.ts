import { isBrowserSurface } from './surface';

export function folderAccessLifetime(): string {
  const managedDesktop =
    !isBrowserSurface() && window.appConfig?.get('BIOROUTER_LOCAL_BACKEND') === true;
  return managedDesktop
    ? 'Session grants expire when revoked, this chat is deleted, or the desktop app quits or restarts.'
    : 'Session grants expire when revoked, this chat is deleted, or the backend restarts. Closing or restarting this interface does not revoke them.';
}
