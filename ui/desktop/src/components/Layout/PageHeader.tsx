import React from 'react';
import { ReadableContent } from './ReadableContent';

type PageHeaderProps = {
  /** The page title. `text-title`, and the only `<h1>` on the page. */
  title: string;
  /**
   * One sentence saying what the page is for. Optional because a drill-in may
   * have nothing to add, but a page that has something to say says it here
   * rather than in a `<Note>` under the hairline.
   */
  description?: React.ReactNode;
  /**
   * The page's actions. Rendered on their OWN LINE under the description, in a
   * `.biorouter-settings-control-strip` — see the block comment below, which is
   * the operator's decision and the reason §4.2 was amended.
   */
  actions?: React.ReactNode;
  /**
   * One extra control under the description and under the actions — Chat
   * history's "Show subagent runs" checkbox. Deliberately NOT a second action
   * slot: a control that changes what the page SHOWS is not an action the page
   * offers, and mixing the two into one strip is how a filter ends up looking
   * like a committing button.
   */
  children?: React.ReactNode;
  /** A badge or count beside the title, on the title's own line. */
  titleAdornment?: React.ReactNode;
};

/**
 * The one page header, for every top-level view: Workflows, the Scheduler,
 * Extensions, Skills, Built apps, Chat history and Settings.
 *
 * Before this existed there were eight copies of the same eleven lines, and
 * they had already drifted in four ways, counted across the eight: the hairline
 * was full-bleed in SEVEN and capped at the reading column in Skills; the
 * description was `text-body` in FIVE and `text-secondary` in THREE; the
 * padding was `px-8` in FIVE and `px-6` in THREE; and the action row was
 * `flex gap-3 mt-5` in FOUR, a right-aligned cluster on the title row in TWO,
 * and absent in TWO. The same five/three split three times over is the tell:
 * it is not eight decisions, it is one header copied twice and then edited.
 * `measures.test.ts` asserts at the source that each of those views imports
 * THIS component, so a ninth view cannot quietly grow a ninth copy.
 *
 * ⚠ **The actions sit on their own line, under the description** (operator
 * decision, 2026-09-07). Astryx §4.2 originally specified the opposite —
 * "actions right-aligned on the title row" — and Chat history and the Scheduler
 * had both been built to it. The operator asked for the button row instead,
 * naming Workflows / Extensions / Skills / Built apps as the shape the rest
 * should match, so §4.2 is amended rather than quietly contradicted; the
 * amendment is dated in `docs/design/astryx-adoption/astryx-ui-adoption-design.md`.
 * The argument for it, beyond consistency: a title row that also carries
 * controls has to give the title `min-w-0 truncate`, so a page title becomes
 * something that can be clipped by its own buttons.
 *
 * ⚠ **The hairline wrapper is OUTSIDE the column, always.** That is what makes
 * the rule run edge to edge rather than stopping at the reading measure — the
 * defect Skills shipped, where the `border-b` sat on the `ReadableContent`
 * itself. Because the hairline is full-bleed, the header's column and the
 * body's column below it must carry the SAME size, or the step between them is
 * visible along the edge they share; `measures.test.ts` asserts that too.
 *
 * The wrapper takes `.biorouter-page-header` — design.md's D-05/P1 flat header,
 * already authored in `main.css` and already worn by the two transcript
 * headers — rather than the `border-b border-border-subtle` pair the eight
 * copies hand-rolled. Measured in the running app the border is identical
 * (`1px solid rgb(48,47,44)` in dark Parchment, both ways); what the authored
 * class adds is the transparent ground and the absent shadow that D-05 also
 * asks for, and it is authored CSS rather than a utility, which is the safer
 * side of the class-scanning trap `.br-swatch-ring` records.
 *
 * ⚠ **No `page-transition` class.** Seven of the eight headers carried one; it
 * matches no CSS rule anywhere in the repo and, measured in the running app,
 * resolves to no animation, no keyframes and no transition. It is the
 * `text-iconStandard` case the vocabulary's rule 4 names — a class that reads as
 * intent and does nothing. `ChatInput.tsx` still applies it conditionally and is
 * out of this PR's scope; a header does not need to inherit it to match.
 */
export function PageHeader({
  title,
  description,
  actions,
  children,
  titleAdornment,
}: PageHeaderProps) {
  return (
    <div className="biorouter-page-header flex-shrink-0">
      <ReadableContent size="chat" className="px-6 pt-12 pb-6">
        <div className="mb-1 flex min-w-0 items-center gap-3">
          <h1 className="text-title min-w-0">{title}</h1>
          {titleAdornment}
        </div>
        {description && <p className="text-secondary text-text-muted">{description}</p>}
        {/* V5 — the same strip Settings uses for every group of section
            buttons, so a page's actions and a section's actions are one shape
            rather than two that nearly match. It wraps, which is why the widest
            of these (Extensions' and Skills' three) survives a narrow pane
            without a horizontal scrollbar. Buttons inside it carry variant and
            size only: V7's `flex` on a Button flips `buttonVariants`' own
            `inline-flex` through tailwind-merge, which is how a row action
            elsewhere became a full-width bar. */}
        {actions && <div className="biorouter-settings-control-strip mt-5">{actions}</div>}
        {children}
      </ReadableContent>
    </div>
  );
}
