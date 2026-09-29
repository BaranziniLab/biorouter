import { StatusDot } from '../../ui/status-dot';
import { identityCopy } from './copy';

/**
 * The people the broker says are online now (W2-BRK-6), as a set, or `null` when it says nothing:
 * an older broker, or a view that is not the verified one (a view kept while offline would name
 * people as online who may not be). Display only: presence is never authority.
 */
export function onlineSet(
  snapshot: { online_principal_ids?: readonly string[] } | null | undefined
): ReadonlySet<string> | null {
  const ids = snapshot?.online_principal_ids;
  return Array.isArray(ids) ? new Set(ids) : null;
}

/**
 * "Online" beside a person's name (M18): a small status dot, with the word for a screen reader and
 * on hover. Renders nothing for someone not online, so a list says only what the broker said.
 */
export function OnlineMark({ online }: { online: boolean }) {
  if (!online) return null;
  return (
    <span className="crew-online-mark" data-crew-online="" title={identityCopy.online}>
      <StatusDot tone="success" />
      <span className="sr-only">{identityCopy.online}</span>
    </span>
  );
}
