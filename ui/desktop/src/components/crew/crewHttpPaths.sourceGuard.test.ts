import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Every value a Crew daemon path interpolates is one encoded path segment (RENDERER-2).
 *
 * `crewHttp` sends the daemon secret and the person's `X-User-Action` proof with every request,
 * whatever the path, and `fetch` normalizes dot segments. Stop task built
 * `/connections/${connectionId}/runs/${runId}/cancel` from a run ID the broker supplied, so an ID
 * of `../../../credentials/lock?` sent the person's proof to `/crew/credentials/lock` instead. So
 * in every `crewHttp` path under `crew/`, each `${…}` before the query string must be an
 * `encodeURIComponent(…)`. A query string is built by `URLSearchParams`, which encodes its own:
 * it follows a literal `?`, or it is a `${query}` that ends the path and carries its own `?`.
 *
 * The detector is run on bad fixtures first, so the guard cannot pass by failing to look.
 */

const CREW_DIR = __dirname;

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(path);
    return [path];
  });
}

const SOURCE_FILES = walk(CREW_DIR).filter(
  (path) =>
    /\.tsx?$/.test(path) && !/\.test\.tsx?$/.test(path) && !path.includes(`${sep}test${sep}`)
);

/** The template-literal first argument of every `crewHttp(…)` / `crewHttp<…>(…)` call. */
function crewHttpPaths(source: string): string[] {
  return [...source.matchAll(/crewHttp(?:<[^()]*?>)?\(\s*`([^`]*)`/g)].map((match) => match[1]);
}

/** The interpolations in a path, before its query string, that are not an encoded segment. */
function unencodedSegments(path: string): string[] {
  const beforeQuery = path.split('?')[0].replace(/\$\{\s*query\s*\}$/, '');
  return [...beforeQuery.matchAll(/\$\{([^}]*)\}/g)]
    .map((match) => match[1].trim())
    .filter((expression) => !/^encodeURIComponent\(/.test(expression));
}

describe('Crew daemon paths (RENDERER-2)', () => {
  it('finds an unencoded segment in a bad fixture, and accepts encoded ones and a query', () => {
    const bad = 'await crewHttp(`/connections/${connectionId}/runs/${runId}/cancel`, "POST", {});';
    expect(crewHttpPaths(bad).flatMap(unencodedSegments)).toEqual(['connectionId', 'runId']);
    const generic = 'crewHttp<{ runs: Run[] }>(`/connections/${id}/runs`)';
    expect(crewHttpPaths(generic).flatMap(unencodedSegments)).toEqual(['id']);
    const midQuery = 'crewHttp(`/connections/${query}/runs`)';
    expect(crewHttpPaths(midQuery).flatMap(unencodedSegments)).toEqual(['query']);
    const good =
      'crewHttp(`/connections/${encodeURIComponent(id)}/runs/${encodeURIComponent(run)}`); ' +
      'crewHttp<{ transfers: T[] }>(`/transfers?${query}`); ' +
      'crewHttp(`/connections/${encodeURIComponent(id)}/invitation${query}`)';
    expect(crewHttpPaths(good)).toHaveLength(3);
    expect(crewHttpPaths(good).flatMap(unencodedSegments)).toEqual([]);
  });

  it('interpolates only encoded segments in every crewHttp path under crew/', () => {
    const offenders: string[] = [];
    let paths = 0;
    for (const file of SOURCE_FILES) {
      for (const path of crewHttpPaths(readFileSync(file, 'utf8'))) {
        paths += 1;
        for (const segment of unencodedSegments(path)) {
          offenders.push(`${relative(CREW_DIR, file).split(sep).join('/')}: ${path} (${segment})`);
        }
      }
    }
    // A scan that reads nothing would agree with a scan that finds nothing.
    expect(paths).toBeGreaterThan(10);
    expect(offenders).toEqual([]);
  });
});
