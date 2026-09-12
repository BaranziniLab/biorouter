import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Every React context the renderer creates must be provided somewhere.
 *
 * A context that is created and read but never wrapped in a `Provider` is not
 * inert: its hook returns the default forever, so every branch downstream that
 * waits for a real value is unreachable code that reads like live code. That is
 * exactly what happened to the composer's model chip. `BaseChat.tsx` created
 * and exported `CurrentModelContext` / `useCurrentModelInfo`, nothing ever
 * provided it, and `ModelsBottomBar` picked which model to name in a
 * lead/worker pair by branching on a value that could never arrive. The label
 * was decided by the fallback in every case the branch was written to handle.
 * Five specs then carried a `vi.mock` of that hook returning `null`, which made
 * the dead value look like a deliberately stubbed one.
 *
 * Measured on `main` at 5a404ecd, before this guard existed: of the twelve
 * contexts created under `src/`, eleven had at least one `Provider` and
 * `CurrentModelContext` had zero — the only one, and the one whose dead branch
 * had already cost a defect (D7 in PR #283).
 *
 * The rule is deliberately blunt: created means provided, with no exemption for
 * a context that carries a real default value. Every context here is provided
 * today, including the one with a non-null default, so such an exemption would
 * only ever wave through the next dead one. If a genuinely default-only context
 * is ever wanted, add it to `PROVIDED_ELSEWHERE` with the reason it can never
 * be provided — that entry is the argument, and it should be arguable.
 *
 * Scope: `src/`, production files only. A `Provider` that appears only in a
 * spec does not rescue a context, because production is where the null arrives.
 */
const SRC = join(__dirname, '..');

/**
 * Trees under `src/` this guard does not read. `api/` is generated from the
 * OpenAPI spec; `bin/` and `web/` are build outputs.
 */
const OUT_OF_SCOPE = ['api/', 'bin/', 'web/'];

/**
 * Contexts created here that can only ever be provided outside this tree, each
 * with the reason. Empty on purpose — see the note above before adding one.
 */
const PROVIDED_ELSEWHERE: { name: string; because: string }[] = [];

interface SourceFile {
  rel: string;
  text: string;
}

function productionSources(): SourceFile[] {
  const found: SourceFile[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      const rel = relative(SRC, path);
      if (statSync(path).isDirectory()) {
        if (!OUT_OF_SCOPE.some((prefix) => `${rel}/`.startsWith(prefix))) walk(path);
        continue;
      }
      if (!/\.tsx?$/.test(entry)) continue;
      if (entry.includes('.test.') || entry.includes('.spec.') || entry.endsWith('.d.ts')) continue;
      if (OUT_OF_SCOPE.some((prefix) => rel.startsWith(prefix))) continue;
      found.push({ rel, text: readFileSync(path, 'utf8') });
    }
  };
  walk(SRC);
  return found;
}

const FILES = productionSources();

/** `const Foo = createContext…` / `const Foo = React.createContext…`, exported or not. */
const DECLARATION =
  /(?:^|\n)[ \t]*(?:export[ \t]+)?const[ \t]+([A-Za-z_$][\w$]*)[ \t]*=[ \t]*(?:React\.)?createContext\b/g;

interface CreatedContext {
  name: string;
  file: string;
  line: number;
}

function createdContexts(): CreatedContext[] {
  const found: CreatedContext[] = [];
  for (const { rel, text } of FILES) {
    for (const match of text.matchAll(DECLARATION)) {
      found.push({
        name: match[1],
        file: rel,
        // The match may start on the newline that precedes the declaration;
        // report the line the `const` is actually on.
        line: text.slice(0, match.index + (match[0].startsWith('\n') ? 1 : 0)).split('\n').length,
      });
    }
  }
  return found;
}

/**
 * Both ways a context can be provided in React 19: the classic
 * `<Foo.Provider value={…}>`, and the shorthand `<Foo value={…}>` that renders
 * the context object itself. The shorthand is not used in this tree yet;
 * accepting it keeps the guard from going red on a correct future refactor.
 */
function isProvided(name: string): boolean {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const provider = new RegExp(`\\b${escaped}\\.Provider\\b`);
  const shorthand = new RegExp(`<${escaped}\\s+value\\s*=`);
  return FILES.some(({ text }) => provider.test(text) || shorthand.test(text));
}

describe('React contexts in the renderer', () => {
  it('finds the contexts to check', () => {
    // A guard whose scan silently matched nothing would pass forever. This tree
    // had twelve contexts when the guard was written and will not drop to zero.
    expect(createdContexts().length).toBeGreaterThanOrEqual(10);
  });

  it('provides every context it creates', () => {
    const exempt = new Set(PROVIDED_ELSEWHERE.map(({ name }) => name));
    const orphans = createdContexts()
      .filter(({ name }) => !exempt.has(name))
      .filter(({ name }) => !isProvided(name))
      .map(({ name, file, line }) => `${file}:${line} ${name}`);

    expect(orphans).toEqual([]);
  });
});
