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
 *   saved file should not carry U+FFFD in its name, along with private-use characters and lone
 *   surrogates, and `undefined` when nothing is left.
 *
 * Neither changes what is sent anywhere. A transfer's own name (the local file it reads or
 * writes), a path that is copied and an ID all stay exactly as they are.
 */

/** U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR: neither a control nor a format character. */
const LINE_OR_PARAGRAPH_SEPARATOR = /^[\p{Zl}\p{Zp}]$/u;
/**
 * Letters and symbols that draw a blank (FILES2-N1): every default-ignorable code point (the
 * Hangul fillers U+115F, U+1160, U+3164 and U+FFA0 among them, which are letters and so neither
 * controls nor format characters) and U+2800 BRAILLE PATTERN BLANK. Sixty U+3164 between
 * `q3-report.pdf` and `.exe` drew nothing, and pushed the real extension off the card's end.
 */
const BLANK_LOOKING = /^[\p{Default_Ignorable_Code_Point}\u2800]$/u;
/** What is left of `White_Space` once controls and separators are gone: the space separators. */
const WHITE_SPACE_RUN = /\p{White_Space}{2,}/gu;
/** Two or more hidden characters in a row, once each is U+FFFD: drawn as one, as a space run is. */
const HIDDEN_RUN = /\uFFFD{2,}/gu;

/**
 * A control, format, line or paragraph separator character, or one that draws a blank: one that
 * shows nothing of its own.
 */
function isHidden(character: string): boolean {
  return (
    stripHiddenCharacters(character) === '' ||
    LINE_OR_PARAGRAPH_SEPARATOR.test(character) ||
    BLANK_LOOKING.test(character)
  );
}

/**
 * A peer-supplied file name, label or path for display: every hidden character (a bidi override
 * or isolate, a zero-width character, a newline, a line separator, a filler that draws a blank)
 * becomes U+FFFD, a run of them one U+FFFD, and a run of spaces one space, so no run of either can
 * push the end of a name, its extension, out of sight (FILES2-N1). Anything but a string becomes
 * `''`.
 */
export function visibleFileText(raw: unknown): string {
  if (typeof raw !== 'string') return '';
  return Array.from(raw, (character) => (isHidden(character) ? '\uFFFD' : character))
    .join('')
    .replace(HIDDEN_RUN, '\uFFFD')
    .replace(WHITE_SPACE_RUN, ' ');
}

/** Private-use characters draw a glyph of some font's choosing, never one a name can rely on. */
const PRIVATE_USE = /^\p{Co}$/u;
/** A lone surrogate: half of a character, which `Array.from` yields as an element of its own. */
const LONE_SURROGATE = /^\p{Cs}$/u;
/**
 * Every space and dot a name starts with, as one class. `\s` is the set `trim` removes, so taking
 * the spaces and the dots together leaves nothing that a second pass would take: `. .x` is `x`,
 * where trimming, stripping the dots and trimming again gave `.x`, and `. .` gave `.`.
 */
const LEADING_SPACES_AND_DOTS = /^[\s.]+/u;

/**
 * The default name for the native Save dialog, one the daemon accepts (FILES-F3): the name with
 * every hidden and private-use character and every lone surrogate left out, then every space and
 * dot it starts with and every space it ends with (the daemon never saves a dot name into the
 * home, so `.Rprofile` is offered as `Rprofile`, and so is `. .Rprofile`). `undefined` when
 * nothing usable is left (the main process then offers its own default). The main process runs
 * this same function again on whatever it is sent (`crewSaveName` in `utils/crewSharePath.ts`),
 * so this module must stay free of anything only a renderer has.
 *
 * The result is a fixed point: running the rule on its own output changes nothing, which is what
 * lets the main process run it a second time and still propose the renderer's name. It can never
 * be `.` or `..` either. The last line checks that again on purpose: the Save window opens a
 * directory it is given as its default, and resolves `.` and `..` against the main process's
 * working directory, so a name another member chose would pick the folder the window opens in.
 *
 * ⚠ The lone surrogates go in the same per-character filter, before the join, as
 * `utils/untrustedText.ts` requires. Dropping the zero-width character in `a\uDB40\u200B\uDC01b`
 * leaves the two halves adjacent, and the join would fuse them into U+E0001, a tag (format)
 * character, after the only test that could see it; `\uDB80\u200B\uDC00` fuses into U+F0000, a
 * private-use one. Every element that survives the filter is a whole code point, so the join can
 * never make a new one.
 */
export function saveNameFor(raw: unknown): string | undefined {
  if (typeof raw !== 'string') return undefined;
  const name = Array.from(raw)
    .filter(
      (character) =>
        !LONE_SURROGATE.test(character) && !isHidden(character) && !PRIVATE_USE.test(character)
    )
    .join('')
    .replace(LEADING_SPACES_AND_DOTS, '')
    .trimEnd();
  return name && name !== '.' && name !== '..' ? name : undefined;
}
