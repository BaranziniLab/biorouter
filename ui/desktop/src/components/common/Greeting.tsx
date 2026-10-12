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

/**
 * Home has no tab, so it has no lifetime to remember. Its greeting fades in on
 * the FIRST arrival of this window's life and is at rest on every later one
 * (principle 9: a surface that mounts because of navigation appears at rest).
 * The vision review measured the old behaviour replaying on every return to
 * Home, Crew to Home included.
 *
 * The flag is mirrored into `sessionStorage`, which a renderer reload keeps
 * and a new window does not: a reload is also "appears at rest", while a new
 * window is an arrival of its own and fades once. Storage that throws (blocked
 * site data) falls back to the module flag, which still holds for the life of
 * the renderer.
 */
const WINDOW_ARRIVAL_KEY = 'biorouter-greeting-arrived';
let windowArrived = false;

function windowArrivalPending(): boolean {
  if (windowArrived) return false;
  try {
    if (window.sessionStorage.getItem(WINDOW_ARRIVAL_KEY) !== null) {
      windowArrived = true;
      return false;
    }
  } catch {
    // Blocked storage: the module flag decides.
  }
  return true;
}

function markWindowArrival() {
  windowArrived = true;
  try {
    window.sessionStorage.setItem(WINDOW_ARRIVAL_KEY, '1');
  } catch {
    // Blocked storage: the module flag still holds for this renderer.
  }
}

/**
 * Tests only: forget that this window's untabbed greeting has arrived. With
 * `{ reload: true }` only the module flag goes, as it does when the renderer
 * reloads and `sessionStorage` survives.
 */
export function resetGreetingArrivalForTests({ reload = false }: { reload?: boolean } = {}) {
  windowArrived = false;
  if (reload) return;
  try {
    window.sessionStorage.removeItem(WINDOW_ARRIVAL_KEY);
  } catch {
    // Nothing to forget.
  }
}

/**
 * The arrival sentence, in the `title` role (24/32/400). Home and an empty chat
 * each pass their own class (Home adds `text-wrap: balance`, the empty chat
 * centres it); the default is the role alone.
 *
 * Motion: one 175ms fade of the whole line (`useTextAnimator`), and only on an
 * arrival. A chat tab fades on its first appearance and is still when the same
 * tab remounts (a split, a move between panes, a return from Settings); Home
 * fades once per window. Every remount draws a new sentence all the same: the
 * rotation is product voice, and changing words is not motion.
 */
export function Greeting({ className = 'text-title', animate = true, tabId }: GreetingProps) {
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
  // Read here, recorded in the layout effect below: an initialiser must stay
  // pure, because React may call it twice.
  const [firstAppearance] = useState(() => (tabId ? !lifetime.shown : windowArrivalPending()));
  const { message } = lifetime;
  const messageRef = useTextAnimator({ text: message, enabled: animate && firstAppearance });
  useLayoutEffect(() => {
    lifetime.shown = true;
    if (!tabId) markWindowArrival();
  }, [lifetime, tabId]);

  // The sentence is one text node, so the heading's name is the sentence. (The
  // per-character unroll this replaced needed an `aria-label` here and an
  // `aria-hidden` span, or the heading was announced letter by letter.)
  return (
    <h1 className={className}>
      <span ref={messageRef}>{message}</span>
    </h1>
  );
}
