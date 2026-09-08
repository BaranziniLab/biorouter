import React from 'react';

type ReadableContentProps = {
  children: React.ReactNode;
  className?: string;
  size?: 'chat' | 'text' | 'wide' | 'graph';
};

/**
 * `chat` is the column the composer and every chat message occupy
 * (`max-w-measure-chat`, see BaseChat.tsx / ChatInput.tsx). A view that sits
 * directly above the composer — Home — must use it, or its edges will not line
 * up. It names the TOKEN rather than a literal precisely so that alignment
 * survives the measure changing.
 *
 * EVERY top-level view reads it now, for two different reasons (operator
 * decision, 2026-09-07; see the `--measure-page` note in main.css). Home's is
 * alignment; every other one's is that it is a column of labelled rows rather
 * than a document, so width past the measure lands between a row's two halves
 * instead of showing more:
 *
 * - **Home** (SessionsInsights.tsx) — the alignment case above: it sits
 *   directly over the composer.
 * - **Settings** (settings/SettingsView.tsx, all three of its boxes) — a
 *   column of labelled rows, so width beyond the measure separates each
 *   control from the label it names instead of showing more.
 * - **Chat history and the two read-only transcripts**
 *   (sessions/SessionListView.tsx, SessionHistoryView.tsx,
 *   SharedSessionView.tsx) — the same row argument as Settings, plus one of
 *   its own: a history row opens the live chat, and a transcript IS a chat, so
 *   both must line up with the column the conversation is read in. The
 *   transcript's old second ceiling — `max-w-4xl`, the 896px replay column —
 *   is deleted rather than converted; one box, one measure.
 * - **The Scheduler** (schedule/SchedulesView.tsx, schedule/ScheduleDetailView.tsx)
 *   — Settings' argument again: the list pairs a schedule with its status and
 *   its actions, the detail pairs a label with the fact it names, so width past
 *   the measure lands between the two halves of every row.
 *
 * - **The component views** (workflows/WorkflowsView.tsx,
 *   extensions/ExtensionsView.tsx, skills/SkillsView.tsx,
 *   applications/ApplicationsView.tsx, apps/AppsView.tsx) — the last five, and
 *   the ones the argument was originally made AGAINST: they were called
 *   document-shaped, on the theory that a wide window buys more columns or more
 *   cards. It buys neither; each is a list of rows with a title on the left and
 *   controls on the right. MCP apps had no reading column at all before this,
 *   just a `px-8` div, so it is the one that gains a measure rather than
 *   changing one.
 *
 * ⚠ **So `text` has no caller left in `components/`** — but it is still the
 * DEFAULT below, which means a `<ReadableContent>` written tomorrow with no
 * `size` silently gets the page measure. That is why `styles/measures.test.ts`
 * asserts the size at the SOURCE, view by view, instead of trusting the
 * default. Neither the size nor `--measure-page` is deleted here: a view that
 * genuinely is a document should still have a measure to reach for, and
 * removing them is a separate decision from this one.
 */
/**
 * ⚠ Every one of these is a CLAMP, not a flat cap, for the reason spelled out
 * on `--measure-chat` in main.css: a fixed pixel ceiling means a wider window
 * buys dead margin instead of content. The floor of each is exactly the value
 * it replaced, so nothing narrow moves — `max-width` cannot force a box wider
 * than its parent — and the percentage resolves against the content pane rather
 * than the viewport, so the sidebar opening does not widen the column.
 *
 * `chat` stays keyed to the token so it and the composer can never drift; the
 * other three are page measures with their own ceilings.
 */
const WIDTH_BY_SIZE: Record<NonNullable<ReadableContentProps['size']>, string> = {
  chat: 'max-w-measure-chat',
  text: 'max-w-[clamp(1120px,88%,1720px)]',
  wide: 'max-w-[clamp(1280px,92%,1920px)]',
  // `graph` is the Knowledge section's canvas measure and is the ONE size here
  // that reads a token rather than restating a clamp: `--measure-graph` is the
  // widest column in the app, and a literal beside two named siblings is the
  // drift `--measure-chat` already had to be rescued from (ui-spec §3.1, §6.2).
  graph: 'max-w-measure-graph',
};

export function ReadableContent({ children, className = '', size = 'text' }: ReadableContentProps) {
  return (
    <div
      data-size={size}
      className={`biorouter-readable-content mx-auto w-full ${WIDTH_BY_SIZE[size]} ${className}`.trim()}
    >
      {children}
    </div>
  );
}
