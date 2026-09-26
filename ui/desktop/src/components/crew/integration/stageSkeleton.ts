/**
 * The channel stage's skeleton, for measuring the real stylesheets in a real layout engine.
 *
 * `paneCover.browser.test.ts` loads `crew-app.css` and the area stylesheets into Chromium over this
 * markup and measures where a covering pane lands, because jsdom evaluates no container query and
 * no grid. That only means something while the markup is the shape `ChannelStage` renders, so
 * `paneCover.test.tsx` renders the real layout and asserts `stageShape()` of the two is equal: a
 * change to the stage's structure fails there, instead of leaving the browser test measuring a
 * page the app no longer draws.
 *
 * Test-only: nothing in the app imports this file.
 */

export interface StageSkeletonOptions {
  /** Whether the connection bar holds a note (an empty bar is `display: none`). */
  note: boolean;
  pane: 'open' | 'closed';
}

/** One element: its tag and its `crew-*` classes, sorted (`div.crew-channel-bar.crew-connection-bar`). */
function elementName(element: Element): string {
  const classes = [...element.classList].filter((name) => name.startsWith('crew-')).sort();
  return [element.tagName.toLowerCase(), ...classes].join('.');
}

/**
 * The stage's structure as the stylesheets see it: every child of `.crew-stage`, and every child
 * of the channel column and of the pane. Ids, inline styles and non-Crew classes are ignored.
 */
export function stageShape(stage: Element): string[] {
  const lines: string[] = [];
  for (const child of Array.from(stage.children)) {
    lines.push(elementName(child));
    if (child.matches('.crew-channel, .crew-pane')) {
      for (const inner of Array.from(child.children)) {
        lines.push(`${elementName(child)} > ${elementName(inner)}`);
      }
    }
  }
  return lines;
}

/** The note's height in the skeleton, so a measurement can say what the bar's row should be. */
export const SKELETON_NOTE_HEIGHT = 48;

/**
 * `ChannelStage`'s markup, reduced to what the stylesheets place: the channel band, the connection
 * bar (a note in it, or empty with no whitespace so `:empty` matches, as React renders it), the
 * channel body with the timeline slot and a composer-sized block, and the details pane.
 */
export function stageSkeleton({ note, pane }: StageSkeletonOptions): string {
  const bar = note
    ? `<div id="note" role="alert" style="height: ${SKELETON_NOTE_HEIGHT}px">The workspace would not stop the task.</div>`
    : '';
  return [
    '<div class="crew-stage" id="stage">',
    '<section class="crew-channel" id="channel">',
    '<header id="header" style="height: var(--chrome-height); margin: 0"><h1 style="margin: 0">general</h1></header>',
    `<div class="crew-connection-bar crew-channel-bar" id="bar">${bar}</div>`,
    '<div class="crew-channel-body" id="body">',
    '<div class="crew-frame-timeline" id="timeline"><p style="margin: 0">Counts are in.</p></div>',
    '<div id="composer" style="flex: none; height: 96px"><textarea aria-label="Message #general"></textarea></div>',
    '</div>',
    '</section>',
    `<aside class="crew-pane" id="pane" data-state="${pane}">`,
    '<div class="crew-pane-content">',
    '<div class="crew-pane-header" id="pane-header">Back to #general</div>',
    '<div class="crew-pane-body" id="pane-body">Access</div>',
    '</div>',
    '</aside>',
    '</div>',
  ].join('');
}
