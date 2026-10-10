import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Rung 0's wiring in BaseChat (spec 3.5).
 *
 * BaseChat cannot be mounted in jsdom (see `BaseChat.privacy.test.tsx`), so the
 * pieces are exercised on their own (`summary/SummaryHeaderButton.test.tsx`,
 * `hooks/useSummaryRail.test.tsx`, `hooks/useSessionTodos.test.tsx`) and what
 * only BaseChat can get wrong, how they are joined, is pinned at its source.
 */
const source = readFileSync(path.join(process.cwd(), 'src', 'components', 'BaseChat.tsx'), 'utf8');

/** The code without comments, so a sentence about a pattern is not the pattern. */
const code = source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

describe('BaseChat: the summary button and the rail', () => {
  it('renders one header button, driven by rung 0 in both of its modes', () => {
    expect(code.match(/<SummaryHeaderButton\b/g) ?? []).toHaveLength(1);
    const start = code.indexOf('<SummaryHeaderButton');
    const element = code.slice(start, code.indexOf('/>', start));
    expect(element).toContain('mode={summaryRail.mode}');
    expect(element).toContain('railShown={summaryRail.shown}');
    expect(element).toContain('railRendered={summaryRail.rendered}');
    expect(element).toContain('onToggleRail={summaryRail.toggle}');
    expect(element).toContain('popoverOpen={reviewOpen}');
    // The button's aria-controls and the rail's id are one value.
    expect(element).toContain('railId={summaryRailId}');
    expect(code).toMatch(/<ChatSummaryRail[\s\S]*?id=\{summaryRailId\}/);
  });

  it('keeps the to-do list live while the popover OR the rail is shown', () => {
    const call = code.slice(
      code.indexOf('useSessionTodos('),
      code.indexOf(');', code.indexOf('useSessionTodos('))
    );
    expect(call).toMatch(/reviewOpen \|\| summaryRail\.shown/);
  });

  it('calls the to-do hook below the rail and above the early return', () => {
    const rail = code.indexOf('useSummaryRail({');
    const todos = code.indexOf('useSessionTodos(');
    const earlyReturn = code.indexOf('if (sessionLoadError)');
    expect(rail).toBeGreaterThan(-1);
    expect(todos).toBeGreaterThan(rail);
    expect(earlyReturn).toBeGreaterThan(todos);
  });

  it('closes the popover when the rail takes its place, or the button turns into the rail’s', () => {
    expect(code).toMatch(
      /if \(summaryRail\.shown \|\| summaryRail\.mode === 'rail'\) setReviewOpen\(false\)/
    );
  });

  it('tells the rail whether a turn is running, so a loaded history appears at rest', () => {
    const call = code.slice(
      code.indexOf('useSummaryRail({'),
      code.indexOf('});', code.indexOf('useSummaryRail({'))
    );
    expect(call).toMatch(/active: summaryActive,/);
    expect(call).toMatch(/chatPhase: summaryRailChatState\.chatPhase,/);
    const state = code.slice(
      code.indexOf('summaryRailChat({'),
      code.indexOf('});', code.indexOf('summaryRailChat({'))
    );
    expect(state).toContain('clean: isCleanConversation,');
    expect(state).toContain('loading: chatState === ChatState.LoadingConversation,');
    expect(state).toContain('running: isRunningState(chatState),');
    expect(code).toMatch(/const summaryActive = summaryRailChatState\.active;/);
  });

  it('starts a new chat’s first turn in the commit that creates it, with the composer’s FLIP (V2-08)', () => {
    const state = code.slice(
      code.indexOf('summaryRailChat({'),
      code.indexOf('});', code.indexOf('summaryRailChat({'))
    );
    // The commit that sets `isCreatingSession` is the one whose composer FLIP
    // leaves the empty state; the rail must already be in the grid then.
    expect(state).toContain('creatingChat: isCreatingSession,');
    // And the FLIP's layout effect runs first, so the rail's glide finds the
    // composer moving and leaves it alone (useColumnGlide's isAnimating).
    const flip = code.indexOf('translate3d(${deltaX}px, ${deltaY}px, 0)');
    expect(flip).toBeGreaterThan(-1);
    expect(flip).toBeLessThan(code.indexOf('useSummaryRail({'));
  });

  it('merges the two split-box styles after both spreads', () => {
    const spreads = code.indexOf('{...summaryRail.splitPaneProps}');
    expect(code.indexOf('{...artifactPanel.splitPaneProps}')).toBeLessThan(spreads);
    expect(code.indexOf('...summaryRail.splitPaneProps.style')).toBeGreaterThan(spreads);
  });
});

describe('BaseChat: the edits it applies for other areas', () => {
  it('moves the composer by translate only, on the motion tokens', () => {
    expect(code).not.toMatch(/scaleX/);
    expect(code).toMatch(/translate3d\(\$\{deltaX\}px, \$\{deltaY\}px, 0\)/);
    expect(code).toMatch(/\{ duration: DUR\.med, easing: EASE_OUT \}/);
  });

  it('draws the Stop & send banner as a neutral Note, with its sentence in an InfoTip', () => {
    expect(code).toMatch(/<Note\s+tone="neutral"/);
    expect(code).toMatch(/summaryCopy\.stopAndSend\.help\[pendingContinuation\.ownership\]/);
    // The classes it used named no token (`border-border`, `bg-muted`).
    expect(code).not.toMatch(/\bborder-border(?![-\w])|\bbg-muted(?![-\w])/);
  });

  it('has no hand-synced header class and no dead composer hook', () => {
    expect(code).not.toContain('HEADER_ACTION_BUTTON_CLASS');
    expect(code).not.toContain('biorouter-composer-view-transition');
    expect(code).not.toMatch(/\bcoherent\b/);
  });

  it('retries a failed load by mounting the chat again, and still offers the way home', () => {
    // The stream's loadSession runs on mount and clears the error first.
    expect(code).toMatch(/<BaseChatContent\s+key=\{loadAttempt\}/);
    expect(code).toMatch(/onRetryLoad=\{\(\) => setLoadAttempt\(/);
    const card = code.slice(code.indexOf('<EmptyState'), code.indexOf('</MainPanelLayout>'));
    expect(card).toMatch(/variant="secondary"\s+size="sm"\s+onClick=\{onRetryLoad\}/);
    expect(card).toContain('summaryCopy.loadError.retry');
    expect(card).toContain('summaryCopy.loadError.action');
  });

  it('stacks the notices above the composer 4px apart and passes ChatInput no animation flag', () => {
    // Spec 3.7: PrivacyTiersOffNote, PinnedModelNote and the crew access bar keep a 4px gap
    // (Home's Hub uses the same), so a notice reads as a quiet line over the card.
    for (const notice of ['PrivacyTiersOffNote', 'PinnedModelNote', 'ChatCrewAccessBar']) {
      const tag = code.match(new RegExp(`<${notice}\\b[^>]*?/>`))?.[0] ?? '';
      expect(tag, notice).toContain('mb-1');
      expect(tag, notice).not.toContain('mb-2');
    }
    // ChatInput ignores it (the route's one fade is useRouteFade), so BaseChat stops reading it.
    expect(code).not.toMatch(/disableAnimation=\{/);
    expect(code).not.toContain('location.state?.disableAnimation');
  });

  it('puts no titlebar padding on the scroll root: the header band already clears it (V2-13)', () => {
    expect(code).toContain("const contentClassName = 'px-1';");
    expect(code).not.toMatch(/'pt-11'/);
    // The band it relies on renders on every path through the chat.
    expect(code.match(/data-preview-area="header"/g) ?? []).toHaveLength(1);
  });

  it('renames through the one shared helper', () => {
    expect(code).toMatch(/await renameSessionOptimistically\(/);
  });
});
