import { useLayoutEffect, useState } from 'react';
import { useTextAnimator } from '../../hooks/use-text-animator';

interface GreetingProps {
  className?: string;
  animate?: boolean;
  /** A tab survives chat remounts, session creation, and movement between panes. */
  tabId?: string;
}

type GreetingLifetime = { message: string; shown: boolean };
// Renderer-local: returning from Settings preserves tabs, while a new window
// has its own registry. Closed tabs are released by ChatGroupsProvider.
const tabGreetings = new Map<string, GreetingLifetime>();

export function retainTabGreetings(tabIds: readonly string[]) {
  const live = new Set(tabIds);
  for (const tabId of tabGreetings.keys()) {
    if (!live.has(tabId)) tabGreetings.delete(tabId);
  }
}

const MESSAGES = [
  'What insights will your data reveal today?',
  'Which connections in the knowledge graph will lead to better care?',
  'What patient story will you uncover in the EHR today?',
  "Which patterns will the knowledge graph unlock for tomorrow's treatments?",
  'What unanswered question in the EHR can we tackle next?',
  "How will today's data bring us closer to a new breakthrough?",
  'Which patient trends are waiting to be discovered in the EHR?',
  'What surprising links might the knowledge graph reveal today?',
  "Which treatment paths can we refine from today's data?",
  'How will your next query shape patient outcomes?',
  'Which health discovery is hidden in your data today?',
  'What clinical journey will your analysis improve today?',
  'What relationships in the data will bring us closer to a cure?',
  'What question will your data answer next?',
  'Which medical mystery might the knowledge graph help solve today?',
] as const;

export function Greeting({
  className = 'mt-1 text-2xl font-semibold tracking-tight',
  animate = true,
  tabId,
}: GreetingProps) {
  const [lifetime] = useState(() => {
    const existing = tabId ? tabGreetings.get(tabId) : undefined;
    if (existing) return existing;
    const created = {
      message: MESSAGES[Math.floor(Math.random() * MESSAGES.length)],
      shown: false,
    };
    if (tabId) tabGreetings.set(tabId, created);
    return created;
  });
  const [firstAppearance] = useState(() => !lifetime.shown);
  const { message } = lifetime;
  const messageRef = useTextAnimator({ text: message, enabled: animate && firstAppearance });
  useLayoutEffect(() => {
    lifetime.shown = true;
  }, [lifetime]);

  // ⚠ The accessible name lives on the `h1`, and the split text is hidden from
  // assistive technology.
  //
  // `split-type` replaces the single text node with one `<div class="char">` per
  // CHARACTER, and emits no ARIA of its own. Without this, the app's only
  // orienting heading on an empty chat is announced letter by letter - "W h a t
  // i n s i g h t s …" - and heading navigation and word-level review are both
  // broken. It happens on every arrival for anyone who has not turned on
  // reduced motion, which is the majority.
  //
  // `aria-label` rather than a visually-hidden duplicate: the visible text is
  // the same string, so a second copy in the DOM would be one more thing to
  // keep in sync for no gain. `aria-hidden` on the span is the half that
  // matters - the label alone would not stop the character soup being read as
  // the heading's content.
  return (
    <h1 className={className} aria-label={message}>
      <span ref={messageRef} aria-hidden="true">
        {message}
      </span>
    </h1>
  );
}
