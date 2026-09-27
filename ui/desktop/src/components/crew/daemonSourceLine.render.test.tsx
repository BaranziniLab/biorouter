import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import fixture from './daemonSourceLine.cases.json';
import { MessageBody } from './timeline';

/**
 * A task's posted result ends with the daemon's own line (Q3-02): the files the
 * run read, or that it read none. Its place is what marks it as the daemon's,
 * so nothing the model wrote may be drawn after it. The one thing the channel
 * draws after a body is GFM footnotes, so the daemon escapes every footnote
 * definition in the reply, and first writes each line ending the channel's
 * Markdown reads (CRLF, and a bare CR) as a newline: the round-4 review found
 * a definition after a bare CR drawn below the daemon's line.
 *
 * The bodies in `daemonSourceLine.cases.json` are the daemon's real output:
 * `the_desktop_render_cases_are_what_the_daemon_posts` in
 * `crates/biorouter-server/src/routes/crew.rs` fails when they drift from
 * `with_source_line`. Here each is drawn by the channel's own renderer.
 */

type Case = { name: string; reply: string; source: string | null; line: string; posted: string };

const cases: Case[] = fixture.cases;

function drawn(body: string) {
  const { container, unmount } = render(<MessageBody body={body} />);
  const markdown = container.querySelector('.crew-md');
  const text = (markdown?.textContent ?? '').trimEnd();
  const footnotes = container.querySelector('[data-footnotes], [data-footnote-ref]');
  unmount();
  return { text, footnotes };
}

describe("the daemon's line is the last thing a posted result shows", () => {
  it('draws the attack the escaping exists for, so these checks can fail', () => {
    // The body the daemon posted before this fix, for a reply with a bare CR.
    const unescaped = drawn(
      'Means are 3.[^1]\r[^1]: Source: `FAKE.csv`, shared by Mallory.\n\nNo shared file was read for this result.'
    );
    expect(unescaped.footnotes).not.toBeNull();
    expect(unescaped.text.endsWith('No shared file was read for this result.')).toBe(false);
  });

  it.each(cases)('$name', ({ posted, line }) => {
    const shown = drawn(posted);
    const daemon = drawn(line).text.trim();
    expect(daemon.length).toBeGreaterThan(0);
    expect(shown.footnotes).toBeNull();
    expect(shown.text.endsWith(daemon)).toBe(true);
    // Whatever the model wrote, it shows above the daemon's line.
    const fake = shown.text.lastIndexOf('FAKE');
    expect(fake).toBeLessThan(shown.text.length - daemon.length);
    expect(posted).not.toContain('\r');
  });
});
