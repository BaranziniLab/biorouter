import { stripHiddenCharacters } from '../../../utils/untrustedText';

/**
 * File names, server-path labels and server paths as Crew shows them (RENDERER-1).
 *
 * Another member chose every one of these strings, and the broker refuses only control characters
 * (`char::is_control`, category Cc) in a name or a label. A format character got through: a file
 * named `q3_{U+202E}fdp.terminal` (U+202E RIGHT-TO-LEFT OVERRIDE before `fdp`) read `q3_lanimret.pdf`
 * on its card, in its tooltip, in every control named for it, and as the Save dialog's default
 * name, so it saved as what looked like a PDF. People, teams, channels and workspaces were already
 * sanitized for this reason (`identity/displayText.ts`); the files area's strings were not.
 *
 * - {@link visibleFileText} is for display, aria-labels included. A hidden character becomes
 *   U+FFFD, the same rule the native share dialog applies to a dropped file's name and path
 *   (`visibleText` in `utils/crewSharePath.ts`, which this mirrors because that module is
 *   main-process code the renderer cannot import; `peerFileNames.test.tsx` holds the two
 *   together). The name then looks odd instead of looking like a different name.
 * - {@link saveNameFor} is the Save dialog's default name: the same characters left out, since a
 *   saved file should not carry U+FFFD in its name, and `undefined` when nothing is left.
 *
 * Neither changes what is sent anywhere. A transfer's own name (the local file it reads or
 * writes), a path that is copied and an ID all stay exactly as they are.
 */

/** U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR: neither a control nor a format character. */
const LINE_OR_PARAGRAPH_SEPARATOR = /^[\p{Zl}\p{Zp}]$/u;
/** What is left of `White_Space` once controls and separators are gone: the space separators. */
const WHITE_SPACE_RUN = /\p{White_Space}{2,}/gu;

/** A control, format, line or paragraph separator character: one that draws nothing of its own. */
function isHidden(character: string): boolean {
  return stripHiddenCharacters(character) === '' || LINE_OR_PARAGRAPH_SEPARATOR.test(character);
}

/**
 * A peer-supplied file name, label or path for display: every hidden character (a bidi override
 * or isolate, a zero-width character, a newline, a line separator) becomes U+FFFD, and a run of
 * spaces becomes one space. Anything but a string becomes `''`.
 */
export function visibleFileText(raw: unknown): string {
  if (typeof raw !== 'string') return '';
  return Array.from(raw, (character) => (isHidden(character) ? '�' : character))
    .join('')
    .replace(WHITE_SPACE_RUN, ' ');
}

/** Private-use characters draw a glyph of some font's choosing, never one a name can rely on. */
const PRIVATE_USE = /^\p{Co}$/u;

/**
 * The default name for the native Save dialog, one the daemon accepts (FILES-F3): the name with
 * every hidden and private-use character left out, trimmed, and without a leading dot (the daemon
 * never saves a dot name into the home, so `.Rprofile` is offered as `Rprofile`). `undefined`
 * when nothing usable is left (the main process then offers its own default). The main process
 * applies the same rule again to whatever it is sent (`crewSaveName` in `utils/crewSharePath.ts`).
 */
export function saveNameFor(raw: unknown): string | undefined {
  if (typeof raw !== 'string') return undefined;
  const name = Array.from(raw)
    .filter((character) => !isHidden(character) && !PRIVATE_USE.test(character))
    .join('')
    .trim()
    .replace(/^\.+/, '')
    .trim();
  return name || undefined;
}
