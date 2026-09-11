import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * SD-8 on a delegated subagent's tab, the half that lives in BaseChat.
 *
 * BaseChat cannot be mounted in jsdom (see `BaseChat.privacy.test.tsx`), so its
 * obligation is pinned against its source, as the other BaseChat suites do.
 * The slot's own decision — the composer on the desktop, the reason in its
 * place in a browser — is exercised by `subagent/SubagentComposerSlot.test.tsx`;
 * what that suite cannot see is whether BaseChat routes the real composer
 * THROUGH the slot. A composer mounted beside it instead would keep every
 * refused control on screen while the slot's tests stayed green.
 */

/** vitest runs with `ui/desktop` as its root — the idiom the other suites use. */
const source = readFileSync(path.join(process.cwd(), 'src', 'components', 'BaseChat.tsx'), 'utf8');

/** The body of `renderChatInput`, from its arrow to the next top-level const. */
function renderChatInputBody(): string {
  const start = source.indexOf('const renderChatInput = () => (');
  expect(start, 'BaseChat no longer defines renderChatInput').toBeGreaterThan(-1);
  const end = source.indexOf('\n  );\n', start);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe("BaseChat — a subagent's composer in a browser", () => {
  it('mounts ChatInput only inside the read-only slot', () => {
    const body = renderChatInputBody();
    const open = body.indexOf('<SubagentComposerSlot isSubagentChat={isSubagentChat}>');
    const close = body.indexOf('</SubagentComposerSlot>');
    const composer = body.indexOf('<ChatInput');
    expect(open, 'the composer is not wrapped in SubagentComposerSlot').toBeGreaterThan(-1);
    expect(composer).toBeGreaterThan(open);
    expect(close).toBeGreaterThan(composer);
    // One composer, and it is the wrapped one.
    expect(body.split('<ChatInput').length - 1).toBe(1);
  });

  it('puts the continuation banner inside the slot too', () => {
    // Its Take over and Abandon post the continuation routes, which SD-11
    // refuses for a subagent's chat on a keyless daemon, exactly like Stop.
    const body = renderChatInputBody();
    const open = body.indexOf('<SubagentComposerSlot');
    expect(body.indexOf('recoverPendingContinuation(')).toBeGreaterThan(open);
    expect(body.indexOf('recoverPendingContinuation(')).toBeLessThan(
      body.indexOf('</SubagentComposerSlot>')
    );
  });

  it('decides from the tab badge first, then either read of the chat', () => {
    const decision = /const isSubagentChat =([\s\S]*?);/.exec(source);
    expect(decision, 'BaseChat no longer computes isSubagentChat').not.toBeNull();
    // ⚠ The badge is the half that matters while the child RUNS. The two reads
    // are ordinary requests, and in a browser they queue behind every open
    // event stream (six connections per origin) — measured at five seconds and
    // more, all of it with the ordinary composer and its Stop on screen. The
    // badge is recorded when the daemon opens the tab, so it is there at mount.
    expect(decision![1]).toMatch(/tabAnnotations\?\.\[sessionId\]\?\.badge === 'subagent'/);
    expect(decision![1]).toMatch(/session\?\.session_type === 'sub_agent'/);
    expect(decision![1]).toMatch(/subagent\.isSubagent/);
  });

  it("tells the transcript's activity nudge that this tab cannot stop the turn", () => {
    // With the composer gone, "You can stop the turn from the composer" would
    // send the reader to a control that is not there.
    expect(source).toMatch(
      /const subagentTabReadOnly = isSubagentChat && subagentTabReadOnlyReason\(\) !== null;/
    );
    const list = /<ProgressiveMessageList\b[\s\S]*?\/>/.exec(source);
    expect(list, 'BaseChat no longer renders ProgressiveMessageList').not.toBeNull();
    expect(list![0]).toMatch(/canStopTurn=\{!subagentTabReadOnly\}/);
  });
});
