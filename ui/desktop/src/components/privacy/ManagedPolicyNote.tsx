import { Note } from '../ui/note';
import { dictationMessage } from '../dictation/copy';
import type { DictationStatus } from '../dictation/dictationApi';

/**
 * What an organization's managed policy turns off for dictation on this computer, in visible
 * text (DD-66 "Managed policy", DD-92). The pattern of `HostManagedModelNote.tsx`: one component
 * owns the sentences so no surface words them twice, and it renders nothing when there is nothing
 * to say, so a call site can mount it unconditionally.
 *
 * The sentences are the daemon's, from `GET /dictation/status`'s `policy` block (the `reason` of
 * each refused door), and the renderer's catalog copy only when a reason is missing. The interface
 * learns policy from that block alone, never from `GET /config`; the daemon checks policy on every
 * request whatever this note says (`dictationStore.ts`).
 *
 * Every privacy control is accident prevention for cooperative actors, never security (DD-92):
 * this note tells a person why a control is locked; it does not lock anything.
 */
export function managedPolicySentences(policy: DictationStatus['policy']): string[] {
  if (!policy.managed) return [];
  if (!policy.enabled.allowed) return [policy.enabled.reason ?? dictationMessage('MSG-POLICY')];
  const sentences: string[] = [];
  if (!policy.capture.allowed) {
    sentences.push(policy.capture.reason ?? dictationMessage('MSG-POLICY-CAPTURE'));
  }
  if (!policy.install.allowed) {
    sentences.push(policy.install.reason ?? dictationMessage('MSG-POLICY-DOWNLOADS'));
  }
  if (policy.allowed_models) {
    sentences.push(
      dictationMessage('MSG-POLICY-MODELS', {
        models: policy.allowed_models.length > 0 ? policy.allowed_models.join(', ') : 'none',
      })
    );
  }
  return sentences;
}

export function ManagedPolicyNote({
  policy,
  className,
}: {
  policy: DictationStatus['policy'];
  /** Layout only. */
  className?: string;
}) {
  const sentences = managedPolicySentences(policy);
  if (sentences.length === 0) return null;
  return (
    <Note tone="neutral" testId="managed-policy-note" className={className}>
      {sentences.join(' ')}
    </Note>
  );
}
