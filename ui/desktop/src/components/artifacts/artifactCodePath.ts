// Whether a file is CODE, decided by the table the code view highlights with.
//
// The preview tab's glyph (`iconForArtifact`, ArtifactViewer.tsx) draws the code
// glyph for code and the document glyph for prose, and "what counts as code" is not
// a second list: a file is code when `languageFromPath` (the table the highlighter
// is given) resolves it to a language that is neither prose nor a table and that the
// highlighter knows. The glyph and the colour therefore cannot disagree about a
// `.R`, a `.toml` or a `.sh` (V2-R3-3).
import { Prism } from 'react-syntax-highlighter';
import { languageFromPath } from './artifactUtils';

/**
 * Languages the code view resolves a file to that are not code: prose, plain text,
 * logs and the two raw-table grammars (`styles/prismGrammars.ts`). They read as a
 * document, so they keep the document glyph.
 */
const NOT_CODE_LANGUAGES = new Set(['text', 'markdown', 'log', 'csv-table', 'tsv-table']);

/**
 * The highlighter's canonical language names. `Prism.supportedLanguages` leaves out
 * Prism's aliases, and one the code view really does map a file to is `xml`
 * (`svg` resolves to it, and a bare `.xml` falls through to it), so it is named.
 */
const CODE_LANGUAGES = new Set<string>([...Prism.supportedLanguages, 'xml']);

/** Whether the code view would highlight this file as code (not prose, not a table). */
export function isCodePath(path: string): boolean {
  const language = languageFromPath(path);
  return !NOT_CODE_LANGUAGES.has(language) && CODE_LANGUAGES.has(language);
}
