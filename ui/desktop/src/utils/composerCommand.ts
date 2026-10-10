/**
 * Slash commands the composer draws as a chip instead of as text (`/bug`).
 *
 * ## The wire format does not change
 *
 * A chip command is still sent as `/bug <prose>` at the START of the message,
 * because the daemon's `execute_command` only recognises a command there. The
 * chip is presentation only: the message text stays the composer's single
 * source of truth (see `composerRefs.ts`), and this module splits its body into
 * the command the rail shows and the prose the textarea shows. Reference tags
 * are unaffected: they stay appended at the end, and `splitComposerText` takes
 * them off before this module sees the body.
 *
 * ## Only `/bug` followed by exactly one space is claimed
 *
 * {@link splitLeadingCommand} claims a lowercase `/bug` at index 0 together with
 * the one space after it, and {@link joinLeadingCommand} writes that space back,
 * so `join(split(x)) === x` for every body. A bare `/bug` with no space stays
 * text. Claiming it would change the message on the next keystroke (`/bug` →
 * `/bug ` once the prose is joined back), and it would turn `/bugfix` into a
 * chip the moment the `g` was typed. The space is what says the command word is
 * finished, as it is for the daemon. `composerRefs.ts` explains why a round trip
 * that adds a character the user did not type moves the caret.
 */

export interface ChipCommandDef {
  /** What the chip reads as. */
  label: string;
  /** What the chip says on hover. */
  title: string;
  /** The textarea's placeholder while the chip is present and the prose is empty. */
  placeholder: string;
}

/** The commands drawn as a chip, keyed by name (no leading `/`). */
export const CHIP_COMMANDS = {
  bug: {
    label: 'Report a bug',
    title: '/bug: Biorouter investigates this chat and drafts an issue for you to approve',
    placeholder: 'Describe what went wrong (optional)',
  },
} as const satisfies Record<string, ChipCommandDef>;

export type ChipCommand = keyof typeof CHIP_COMMANDS;

const CHIP_COMMAND_NAMES = Object.keys(CHIP_COMMANDS) as ChipCommand[];

/** Whether `name` (no leading `/`) is a chip command. Case-sensitive, as the daemon is. */
export const isChipCommand = (name: string): name is ChipCommand =>
  Object.prototype.hasOwnProperty.call(CHIP_COMMANDS, name);

/** `/bug`: the command as typed. */
export const commandToken = (command: ChipCommand): string => `/${command}`;

export interface LeadingCommand {
  /** The chip command the body starts with, or `null`. */
  command: ChipCommand | null;
  /** The body without the command and its one separating space. What the textarea shows. */
  prose: string;
}

/** Split a composer body into the chip command it starts with and the prose after it. */
export function splitLeadingCommand(body: string): LeadingCommand {
  for (const command of CHIP_COMMAND_NAMES) {
    const prefix = `${commandToken(command)} `;
    if (body.startsWith(prefix)) return { command, prose: body.slice(prefix.length) };
  }
  return { command: null, prose: body };
}

/** The composer body for a chip command (or none) and the prose after it. */
export function joinLeadingCommand(command: ChipCommand | null, prose: string): string {
  return command ? `${commandToken(command)} ${prose}` : prose;
}

/**
 * The chip command a picked slash-menu row inserts, if `inserted` is exactly
 * that insert (`/bug `). Anything else is text to splice into the prose.
 */
export function chipCommandOfInsert(inserted: string): ChipCommand | null {
  const { command, prose } = splitLeadingCommand(inserted);
  return command && prose === '' ? command : null;
}

export interface SentCommand {
  /** The chip command a sent message starts with, or `null`. */
  command: ChipCommand | null;
  /** Everything after the command word, the separator included. */
  rest: string;
}

/**
 * The chip command a SENT message starts with, for the transcript and the queue.
 *
 * Wider than {@link splitLeadingCommand} because nothing is typed back into it:
 * it also claims a bare `/bug` (a chip sent with no prose is trimmed to that)
 * and `/bug` followed by any whitespace, which is how the daemon splits the
 * command word off. The separator stays in `rest`, so the prose reads as typed.
 */
export function splitSentCommand(text: string): SentCommand {
  for (const command of CHIP_COMMAND_NAMES) {
    const token = commandToken(command);
    if (text === token || (text.startsWith(token) && /\s/.test(text[token.length]))) {
      return { command, rest: text.slice(token.length) };
    }
  }
  return { command: null, rest: text };
}

/**
 * A sent message as composer text: a bare chip command gets its space back.
 *
 * A `/bug` chip sent with no prose is trimmed to a bare `/bug` on its way out
 * (the send, the chat's command history, the queue all trim), and
 * {@link splitLeadingCommand} leaves a bare `/bug` as text, so that typing
 * `/bugfix` never turns into a chip mid-word. Text the person did NOT type —
 * a recalled history entry, a message handed back to the composer, the edit
 * box's starting text — goes through here first, so it comes back as the chip
 * it was sent as. Anything else is returned unchanged.
 */
export function composerTextOfSent(text: string): string {
  const { command, rest } = splitSentCommand(text);
  return command && rest === '' ? joinLeadingCommand(command, '') : text;
}
