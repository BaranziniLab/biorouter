import { useLayoutEffect, useRef, type RefObject } from 'react';
import type { Message } from '../../api';
import { ChatState } from '../../types/chatState';
import { getToolConfirmationContent } from '../../types/message';
import type { ScrollAreaHandle } from '../ui/scroll-area';

interface PermissionRequestRevealOptions {
  sessionId: string;
  messages: readonly Message[];
  chatState: ChatState;
  transcriptMounted: boolean;
  scrollRef: RefObject<ScrollAreaHandle | null>;
}

export function usePermissionRequestReveal({
  sessionId,
  messages,
  chatState,
  transcriptMounted,
  scrollRef,
}: PermissionRequestRevealOptions): void {
  let confirmationId: string | undefined;
  for (let index = messages.length - 1; index >= 0; index--) {
    const confirmation = getToolConfirmationContent(messages[index]);
    if (confirmation) {
      confirmationId = confirmation.data.id;
      break;
    }
  }

  const observedRef = useRef<{ sessionId: string; confirmationId?: string } | null>(null);
  const pendingRef = useRef<string | undefined>(undefined);

  useLayoutEffect(() => {
    const previous = observedRef.current;
    observedRef.current = { sessionId, confirmationId };
    // Opening a saved transcript establishes a baseline, not a new request.
    if (!previous || previous.sessionId !== sessionId) {
      pendingRef.current = undefined;
      return;
    }
    if (chatState !== ChatState.WaitingForUserInput) {
      pendingRef.current = undefined;
      return;
    }
    if (confirmationId && confirmationId !== previous.confirmationId) {
      pendingRef.current = confirmationId;
    }
    if (!transcriptMounted || !confirmationId || pendingRef.current !== confirmationId) return;

    // Wait for the committed card's layout; only the transcript viewport moves.
    const frame = requestAnimationFrame(() => {
      const scroll = scrollRef.current;
      if (!scroll) return;
      scroll.scrollToBottom('auto');
      pendingRef.current = undefined;
    });
    return () => cancelAnimationFrame(frame);
  }, [sessionId, confirmationId, chatState, transcriptMounted, scrollRef]);
}
