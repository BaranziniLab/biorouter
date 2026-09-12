import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Every renderer call to the active-work routes carries the person's proof.
 *
 * Issue #56: `GET /active_work` now shows a row only to a caller that could open
 * the chat the row belongs to, and `POST /active_work/{id}/cancel` refuses the
 * rest with the chat read's own 403. The desktop is the person at the keyboard,
 * but the daemon only believes that when the request carries
 * `userActionHeaders()`.
 *
 * ⚠ **A missing proof is not an error here, which is why this is a test and not
 * a code review note.** The list comes back 200 with the private chats' work
 * silently left out, so a panel built on it would tell the user nothing is
 * running while their private chat's job still is. `CLAUDE.md` records the same
 * trap for the chat and knowledge-base listings.
 *
 * Measured 2026-09-11: nothing in the renderer calls either route yet (the panel
 * is deferred). So this guard fails the day one is added without the proof, not
 * today. Its positive controls below show that it can fail.
 */
const SRC = __dirname;

/** The generated client's functions for the two routes (`src/api/sdk.gen.ts`). */
const GATED_CALLS = ['listActiveWork', 'cancelActiveWork'] as const;

/** `api/` is generated; `bin/` and `web/` are build outputs. */
const OUT_OF_SCOPE = ['api/', 'bin/', 'web/'];

function productionSources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const rel = relative(SRC, path).replace(/\\/g, '/');
    if (statSync(path).isDirectory()) {
      if (
        entry !== 'node_modules' &&
        !OUT_OF_SCOPE.some((prefix) => `${rel}/`.startsWith(prefix))
      ) {
        productionSources(path, out);
      }
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
      out.push(path);
    }
  }
  return out;
}

/** The source with comments and import declarations blanked, so neither reads as a call. */
function codeOf(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/(^|[^:])\/\/.*$/gm, '$1')
    .replace(/^\s*import\s[^;]*?from\s*['"][^'"]+['"];?/gm, ' ');
}

/** The argument text of the call whose `(` is at `open`, by bracket matching. */
function argumentsAt(code: string, open: number): string {
  let depth = 0;
  for (let i = open; i < code.length; i += 1) {
    if (code[i] === '(') depth += 1;
    if (code[i] === ')') {
      depth -= 1;
      if (depth === 0) return code.slice(open + 1, i);
    }
  }
  return code.slice(open + 1);
}

/**
 * Every place `source` reaches an active-work route without the proof: a call
 * to a gated function whose arguments do not call `userActionHeaders()`, the
 * function passed as a value (its headers cannot be seen, so it cannot be
 * trusted to send them), or a hand-built request to the path.
 */
function unprovenActiveWorkCalls(source: string): string[] {
  const code = codeOf(source);
  const findings: string[] = [];
  for (const name of GATED_CALLS) {
    for (const match of code.matchAll(new RegExp(`\\b${name}\\b\\s*(\\()?`, 'g'))) {
      const open = (match.index ?? 0) + match[0].length - 1;
      if (match[1] === undefined) {
        findings.push(`${name} used as a value`);
      } else if (!/\buserActionHeaders\s*\(/.test(argumentsAt(code, open))) {
        findings.push(`${name}(…) without userActionHeaders()`);
      }
    }
  }
  if (/['"`][^'"`]*\/active_work\b/.test(code)) {
    findings.push('a hand-built request to /active_work');
  }
  return findings;
}

describe("the active-work routes are called with the person's proof", () => {
  it('is a guard that can fail (positive controls)', () => {
    expect(unprovenActiveWorkCalls('await listActiveWork({ throwOnError: true });')).toEqual([
      'listActiveWork(…) without userActionHeaders()',
    ]);
    expect(unprovenActiveWorkCalls('cancelActiveWork({ path: { id } });')).toEqual([
      'cancelActiveWork(…) without userActionHeaders()',
    ]);
    expect(unprovenActiveWorkCalls('const load = listActiveWork;')).toEqual([
      'listActiveWork used as a value',
    ]);
    expect(unprovenActiveWorkCalls('await fetch(`${base}/active_work`, { headers });')).toEqual([
      'a hand-built request to /active_work',
    ]);
  });

  it('accepts the shape the rest of the renderer uses, and ignores imports and comments', () => {
    const proven = [
      "import { cancelActiveWork, listActiveWork } from '../api';",
      '// listActiveWork() without a proof, in prose, is not a call',
      'const list = await listActiveWork({ throwOnError: true, headers: await userActionHeaders() });',
      'await cancelActiveWork({',
      '  path: { id: item.id },',
      '  headers: await userActionHeaders(),',
      '});',
    ].join('\n');
    expect(unprovenActiveWorkCalls(proven)).toEqual([]);
  });

  it('holds for every production source in the renderer', () => {
    const sources = productionSources(SRC);
    // A walk that read nothing reports the same empty list as a clean tree.
    // `chatStreamStore.tsx` names `/active_work` in its comments, so it is also
    // the real-world check that prose is not read as a request.
    expect(sources.length).toBeGreaterThan(200);
    const chatStreamStore = sources.find((path) => path.endsWith('chatStreamStore.tsx'));
    expect(chatStreamStore).toBeDefined();
    expect(readFileSync(chatStreamStore!, 'utf8')).toContain('/active_work');

    const findings = sources.flatMap((path) =>
      unprovenActiveWorkCalls(readFileSync(path, 'utf8')).map(
        (finding) => `${relative(SRC, path)}: ${finding}`
      )
    );
    expect(findings).toEqual([]);
  });
});
