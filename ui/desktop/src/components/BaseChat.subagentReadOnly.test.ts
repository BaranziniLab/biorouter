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
    const open = body.indexOf('<SubagentComposerSlot kind={subagentChatKind}>');
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

  it('hands every source of the fact to one three-valued decision', () => {
    const decision = /const subagentChatKind = subagentComposerKind\(\{([\s\S]*?)\}\);/.exec(
      source
    );
    expect(decision, 'BaseChat no longer computes subagentChatKind').not.toBeNull();
    // ⚠ The badge is the half that is known at MOUNT. The two reads are ordinary
    // requests, and in a browser they queue behind every open event stream (six
    // connections per origin) — measured at five seconds and more, all of it
    // with the ordinary composer and its Stop on screen.
    expect(decision![1]).toMatch(/tabAnnotations\?\.\[sessionId\]\?\.badge/);
    expect(decision![1]).toMatch(/loadedSessionType: session\?\.session_type/);
    expect(decision![1]).toMatch(/hookSaysSubagent: subagent\.isSubagent/);
    // The review's finding 3: the badge does not survive a browser reload, so
    // the decision must be able to say "not yet" as well as "no". These two
    // fields are what let it.
    expect(decision![1]).toMatch(/loadedSessionId: session\?\.id/);
    expect(decision![1]).toMatch(/loadFailed:/);
  });

  it('keys the read-only consequences off the slot, not off subagent-ness', () => {
    // Finding 3. `isSubagentChat` is false while the answer is in flight, so a
    // consequence keyed off it is live for exactly the window the composer is
    // withheld in.
    expect(source).toMatch(
      /const subagentTabReadOnly = composerSlotMode\(subagentChatKind\) !== 'composer';/
    );
  });

  it("tells the transcript's activity nudge that this tab cannot stop the turn", () => {
    // With the composer gone, "You can stop the turn from the composer" would
    // send the reader to a control that is not there.
    const list = /<ProgressiveMessageList\b[\s\S]*?\/>/.exec(source);
    expect(list, 'BaseChat no longer renders ProgressiveMessageList').not.toBeNull();
    expect(list![0]).toMatch(/canStopTurn=\{!subagentTabReadOnly\}/);
  });
});
