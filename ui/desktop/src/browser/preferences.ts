import fs from 'node:fs';
import path from 'node:path';
import type { SearchEngine } from './addressBar';

/**
 * The Biorouter Browser's few settings that change how browsing feels (BB-22): the search engine
 * the address field uses, the languages pages are asked for, and the default zoom of a new page.
 * Each is unset until an import from Chrome sets it; unset means the built in default (Google, the
 * system's preferred languages, 100%).
 *
 * One JSON file beside the history, mode 0600. Main reads it; the renderer asks for the search
 * engine only.
 */

export type BrowserPreferences = {
  searchEngine?: SearchEngine;
  acceptLanguages?: string;
  defaultZoomFactor?: number;
};

export type BrowserPreferencesStore = ReturnType<typeof createBrowserPreferences>;

function validSearchEngine(value: unknown): SearchEngine | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const engine = value as Record<string, unknown>;
  if (typeof engine.name !== 'string' || typeof engine.template !== 'string') return undefined;
  if (!engine.template.includes('{searchTerms}') || engine.template.length > 2048) return undefined;
  try {
    const url = new URL(engine.template.replace('{searchTerms}', 'probe'));
    if (url.protocol !== 'https:' || url.username || url.password) return undefined;
  } catch {
    return undefined;
  }
  return { name: engine.name.slice(0, 60), template: engine.template };
}

function clean(raw: unknown): BrowserPreferences {
  const value = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const out: BrowserPreferences = {};
  const engine = validSearchEngine(value.searchEngine);
  if (engine) out.searchEngine = engine;
  if (
    typeof value.acceptLanguages === 'string' &&
    /^[a-z]{2,3}(?:-[a-z0-9]{2,8})*(?:,[a-z]{2,3}(?:-[a-z0-9]{2,8})*){0,7}$/i.test(
      value.acceptLanguages
    )
  ) {
    out.acceptLanguages = value.acceptLanguages;
  }
  const zoom = Number(value.defaultZoomFactor);
  if (Number.isFinite(zoom) && zoom >= 0.25 && zoom <= 5 && Math.abs(zoom - 1) > 1e-6) {
    out.defaultZoomFactor = zoom;
  }
  return out;
}

export function createBrowserPreferences(dir: string | null) {
  const file = dir ? path.join(dir, 'preferences.json') : null;
  let current: BrowserPreferences | null = null;

  const read = (): BrowserPreferences => {
    if (current) return current;
    try {
      current = file ? clean(JSON.parse(fs.readFileSync(file, 'utf8'))) : {};
    } catch {
      current = {};
    }
    return current;
  };

  return {
    get(): BrowserPreferences {
      return { ...read() };
    },
    /** Replace the settings; a key set to `undefined` goes back to the built in default. */
    set(next: BrowserPreferences): BrowserPreferences {
      current = clean(next);
      if (file) {
        try {
          fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
          const temporary = `${file}.${process.pid}.tmp`;
          fs.writeFileSync(temporary, JSON.stringify({ version: 1, ...current }), { mode: 0o600 });
          fs.renameSync(temporary, file);
        } catch {
          // A read only profile keeps the settings for this run only.
        }
      }
      return { ...current };
    },
  };
}
