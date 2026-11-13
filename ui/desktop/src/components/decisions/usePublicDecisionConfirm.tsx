import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import type { PublicDecisionConfirmation } from '../../utils/sessionDecisions';
import { TIER_EFFECT_COPY, publicModelsList } from './decisionWords';
import { TierEffectLine } from './TierEffectLine';

/** Which start is asking: a composer's Send, or a workflow's start. */
export type PublicDecisionAsk = 'send' | 'workflow';

/**
 * The words of the question a start asks when the daemon answers that the chat
 * would be public (409 `needsPublicDecisionConfirmation`, DR-33): a composer's Send
 * asks "Make this chat public?" with the daemon's own line, a workflow "Start this
 * workflow?" with the models it uses (design §5's row for a workflow start).
 */
export function publicDecisionQuestion(
  confirmation: PublicDecisionConfirmation,
  ask: PublicDecisionAsk
): { title: string; message: string; confirmLabel: string } {
  if (ask === 'workflow' && confirmation.models.length > 0) {
    return {
      title: TIER_EFFECT_COPY.workflowTitle,
      message: TIER_EFFECT_COPY.workflowQuestion(publicModelsList(confirmation.models)),
      confirmLabel: TIER_EFFECT_COPY.workflowConfirm,
    };
  }
  return {
    title: TIER_EFFECT_COPY.confirmTitle,
    message: confirmation.message,
    confirmLabel:
      ask === 'workflow' ? TIER_EFFECT_COPY.workflowConfirm : TIER_EFFECT_COPY.confirmSend,
  };
}

/**
 * A promise-shaped "This chat will be public" question, for a start that only
 * learns it must ask once the daemon answers. `confirm` resolves `true` when the
 * person says yes; `dialog` is the element to render (once) beside the caller.
 *
 * It is a notice the person acts on, not a gate of its own: the daemon refuses a
 * lowering without the answer at every door, and this only lets the person give it.
 */
export function usePublicDecisionConfirm(ask: PublicDecisionAsk): {
  confirm: (confirmation: PublicDecisionConfirmation) => Promise<boolean>;
  dialog: ReactNode;
} {
  const [pending, setPending] = useState<PublicDecisionConfirmation | null>(null);
  const resolver = useRef<((answer: boolean) => void) | null>(null);

  const settle = useCallback((answer: boolean) => {
    const resolve = resolver.current;
    resolver.current = null;
    setPending(null);
    resolve?.(answer);
  }, []);

  // An asker that unmounts mid-question answers no: nothing starts behind a closed screen.
  useEffect(() => () => resolver.current?.(false), []);

  const confirm = useCallback((confirmation: PublicDecisionConfirmation) => {
    resolver.current?.(false);
    return new Promise<boolean>((resolve) => {
      resolver.current = resolve;
      setPending(confirmation);
    });
  }, []);

  const words = pending ? publicDecisionQuestion(pending, ask) : null;
  const dialog = (
    <ConfirmationModal
      isOpen={pending !== null}
      title={words?.title ?? TIER_EFFECT_COPY.confirmTitle}
      message={words?.message ?? ''}
      confirmLabel={words?.confirmLabel ?? TIER_EFFECT_COPY.confirmSend}
      cancelLabel={TIER_EFFECT_COPY.confirmCancel}
      onConfirm={() => settle(true)}
      onCancel={() => settle(false)}
    >
      {pending?.info ? (
        <TierEffectLine
          line={TIER_EFFECT_COPY.publicInfoLabel}
          info={pending.info}
          infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
          className="text-text-muted"
          testId="public-decision-confirm-why"
        />
      ) : null}
    </ConfirmationModal>
  );
  return { confirm, dialog };
}
