import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Final acceptance O-1: two renderers were seen on `#/crew` after being left on Home and on a chat,
 * around transfer and grant events, and the lane that saw it had not navigated. The audit found no
 * code that moves a window to Crew on an event — the likeliest cause is a concurrent lane driving
 * the same apps (the novice lane opened Crew in Jack's and Iris's windows in that window of time) —
 * and this census keeps it that way: every place in the renderer that navigates to Crew is listed
 * here, and each is a person's action. A new one fails this test until someone has looked at it.
 *
 * Behaviour is pinned beside this: the ordinary chat's Crew bar stays put through every grant,
 * turn, connection and window event (`access/chatCrewAccess.acceptance.test.tsx`, O-1).
 */

const SRC = join(__dirname, '..', '..', '..');

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(path);
    return /\.(ts|tsx)$/.test(entry.name) && !/\.test\.|\.spec\./.test(entry.name) ? [path] : [];
  });
}

/** A navigation whose destination is Crew: the route, the chat-access route, or the view. */
const TO_CREW =
  /navigate\(\s*['"`]\/crew|navigate\(\s*chatAccessRoute\(|setView\(\s*['"]crew['"]|navigateWithViewTransition\([^)]*['"`]\/crew/;

const rel = (path: string) => relative(SRC, path).split(sep).join('/');

function sites(): string[] {
  return walk(SRC).flatMap((path) =>
    readFileSync(path, 'utf8')
      .split('\n')
      .flatMap((line) => (TO_CREW.test(line) ? [`${rel(path)}: ${line.trim()}`] : []))
  );
}

describe('the renderer moves a window to Crew only when a person asks (O-1)', () => {
  it('has exactly the known navigations to Crew', () => {
    expect(sites().sort()).toEqual(
      [
        // "Open Crew →" beside the provider setup: a click.
        "components/ProviderGuard.tsx: onClick={() => navigate('/crew')}",
        // The `/crew` command, sent with Enter or Send, or picked from the popover.
        "components/ChatInput.tsx: setView('crew', { resumeSessionId: sessionId });",
        // The chat's Crew bar: the chip and Grant access again; Connect in Crew.
        'components/crew/access/ChatCrewAccessBar.tsx: navigate(chatAccessRoute(sessionId), { state: chatAccessRouteState() });',
        'components/crew/access/ChatCrewAccessBar.tsx: navigate(chatAccessRoute(sessionId), { state: chatConnectRouteState(grant.connection_id) });',
      ].sort()
    );
  });

  it('reaches the chat bar’s two navigations only from buttons', () => {
    const source = readFileSync(join(SRC, 'components/crew/access/ChatCrewAccessBar.tsx'), 'utf8');
    for (const name of ['openAccess', 'connectInCrew']) {
      const uses = source.match(new RegExp(`\\b${name}\\b.*`, 'g')) ?? [];
      const [definition, ...rest] = uses;
      expect(definition).toMatch(new RegExp(`^${name} = \\(\\) => \\{`));
      expect(rest.length).toBeGreaterThan(0);
      for (const use of rest) expect(use).toMatch(new RegExp(`^${name}\\}`));
      // …and every one of those sits in an onClick.
      expect(source.match(new RegExp(`onClick=\\{${name}\\}`, 'g'))?.length).toBe(rest.length);
    }
  });

  it('reaches the /crew command only from the person’s Enter, Send or pick', () => {
    const source = readFileSync(join(SRC, 'components/ChatInput.tsx'), 'utf8');
    // No effect, timer or listener calls it: only the key handler, the form's submit, the send
    // path and the popover's pick.
    const effects = source.match(/useEffect\(\(\) => \{[\s\S]*?\n {2}\}, \[/g) ?? [];
    for (const effect of effects) expect(effect).not.toMatch(/\bopenCrew\(/);
    expect(source).not.toMatch(/setTimeout\([^)]*openCrew|addEventListener\([^)]*openCrew/);
  });
});
