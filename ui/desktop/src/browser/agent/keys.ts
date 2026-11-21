/**
 * Keys and chords for `browser_press` and `browser_type` (BB-13), as CDP
 * `Input.dispatchKeyEvent` wants them.
 *
 * Pure, so Vitest pins the mapping and which presses need the act grant.
 */

/** CDP's modifier bit mask. */
export const MODIFIER_BITS = { Alt: 1, Control: 2, Meta: 4, Shift: 8 } as const;
export type Modifier = keyof typeof MODIFIER_BITS;

export type KeyStroke = {
  key: string;
  code: string;
  keyCode: number;
  /** The character the key types, if any (with Shift applied for letters). */
  text?: string;
  modifiers: number;
  /** macOS editing commands a chord must carry, since key events alone do not run them. */
  commands?: string[];
};

const NAMED: Record<string, { key: string; code: string; keyCode: number; text?: string }> = {
  enter: { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' },
  return: { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' },
  tab: { key: 'Tab', code: 'Tab', keyCode: 9 },
  escape: { key: 'Escape', code: 'Escape', keyCode: 27 },
  esc: { key: 'Escape', code: 'Escape', keyCode: 27 },
  backspace: { key: 'Backspace', code: 'Backspace', keyCode: 8 },
  delete: { key: 'Delete', code: 'Delete', keyCode: 46 },
  space: { key: ' ', code: 'Space', keyCode: 32, text: ' ' },
  arrowup: { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
  arrowdown: { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 },
  arrowleft: { key: 'ArrowLeft', code: 'ArrowLeft', keyCode: 37 },
  arrowright: { key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 },
  up: { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
  down: { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 },
  left: { key: 'ArrowLeft', code: 'ArrowLeft', keyCode: 37 },
  right: { key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 },
  home: { key: 'Home', code: 'Home', keyCode: 36 },
  end: { key: 'End', code: 'End', keyCode: 35 },
  pageup: { key: 'PageUp', code: 'PageUp', keyCode: 33 },
  pagedown: { key: 'PageDown', code: 'PageDown', keyCode: 34 },
  insert: { key: 'Insert', code: 'Insert', keyCode: 45 },
};

const MODIFIER_NAMES: Record<string, Modifier> = {
  alt: 'Alt',
  option: 'Alt',
  control: 'Control',
  ctrl: 'Control',
  meta: 'Meta',
  cmd: 'Meta',
  command: 'Meta',
  super: 'Meta',
  shift: 'Shift',
};

/** Single characters a chord can name, with their US layout codes. */
function characterStroke(char: string): { key: string; code: string; keyCode: number } | null {
  if (/^[a-z]$/i.test(char)) {
    const upper = char.toUpperCase();
    return { key: char, code: `Key${upper}`, keyCode: upper.charCodeAt(0) };
  }
  if (/^[0-9]$/.test(char)) return { key: char, code: `Digit${char}`, keyCode: char.charCodeAt(0) };
  const punctuation: Record<string, [string, number]> = {
    '-': ['Minus', 189],
    '=': ['Equal', 187],
    '[': ['BracketLeft', 219],
    ']': ['BracketRight', 221],
    ';': ['Semicolon', 186],
    "'": ['Quote', 222],
    ',': ['Comma', 188],
    '.': ['Period', 190],
    '/': ['Slash', 191],
    '\\': ['Backslash', 220],
    '`': ['Backquote', 192],
  };
  const entry = punctuation[char];
  return entry ? { key: char, code: entry[0], keyCode: entry[1] } : null;
}

/** The macOS editing command a Meta chord runs, which CDP must be told about explicitly. */
const MAC_COMMANDS: Record<string, string> = {
  a: 'selectAll',
  c: 'copy',
  x: 'cut',
  v: 'paste',
  z: 'undo',
};

/**
 * Parse `"Enter"`, `"Meta+A"`, `"Control+Shift+Tab"`. Returns an error sentence for anything
 * that is not one key with optional modifiers.
 */
export function parseChord(
  input: string,
  platform: string
): { ok: true; stroke: KeyStroke } | { ok: false; error: string } {
  const parts = input
    .split('+')
    .map((part) => part.trim())
    .filter(Boolean);
  // "Control++" means the plus key.
  if (input.trim().endsWith('++')) parts.push('+');
  if (parts.length === 0) return { ok: false, error: 'Name a key, such as Enter or Meta+A.' };
  const keyName = parts[parts.length - 1];
  let modifiers = 0;
  for (const part of parts.slice(0, -1)) {
    const modifier = MODIFIER_NAMES[part.toLowerCase()];
    if (!modifier)
      return { ok: false, error: `${part} is not a modifier (Alt, Control, Meta, Shift).` };
    modifiers |= MODIFIER_BITS[modifier];
  }
  const named = NAMED[keyName.toLowerCase()];
  if (named) {
    return { ok: true, stroke: { ...named, modifiers } };
  }
  if ([...keyName].length === 1) {
    const char = keyName;
    const known = characterStroke(char);
    const shift = (modifiers & MODIFIER_BITS.Shift) !== 0;
    const typed = /^[a-z]$/i.test(char) ? (shift ? char.toUpperCase() : char.toLowerCase()) : char;
    const plainTyping = (modifiers & ~MODIFIER_BITS.Shift) === 0;
    const stroke: KeyStroke = {
      key: /^[a-z]$/i.test(char) ? typed : (known?.key ?? char),
      code: known?.code ?? '',
      keyCode: known?.keyCode ?? 0,
      text: plainTyping ? typed : undefined,
      modifiers,
    };
    const meta = (modifiers & MODIFIER_BITS.Meta) !== 0;
    const control = (modifiers & MODIFIER_BITS.Control) !== 0;
    const command = MAC_COMMANDS[char.toLowerCase()];
    if (command && ((platform === 'darwin' && meta) || (platform !== 'darwin' && control))) {
      stroke.commands = [command];
    }
    return { ok: true, stroke };
  }
  return { ok: false, error: `${keyName} is not a key this browser can press.` };
}

/** The keys `browser_type` sends for one character. */
export function strokeForCharacter(char: string): KeyStroke {
  if (char === '\n') return { ...NAMED.enter, modifiers: 0 };
  if (char === '\t') return { ...NAMED.tab, modifiers: 0 };
  const known = characterStroke(char.toLowerCase());
  const upper = /^[A-Z]$/.test(char);
  return {
    key: char,
    code: known?.code ?? '',
    keyCode: known?.keyCode ?? 0,
    text: char,
    modifiers: upper ? MODIFIER_BITS.Shift : 0,
  };
}
