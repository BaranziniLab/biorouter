import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Bot } from '../icons/app-icons';
import {
  AVATAR_HUE_COUNT,
  AVATAR_PAIR_MIN_SIZE,
  Avatar,
  avatarHue,
  avatarInitials,
  avatarTextLimit,
} from './avatar';

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
const THEME_CONTRACT = readFileSync(
  join(__dirname, '../../../scripts/lib/theme-contract.mjs'),
  'utf8'
);

const tile = (container: HTMLElement) =>
  container.querySelector('[data-slot="avatar"]') as HTMLElement;

describe('avatarInitials — the one fallback rule (L16)', () => {
  it.each([
    ['Alice Chen', 'alice', 'A'],
    ['alice', 'alice', 'A'],
    ['Mary Ann Smith', 'msmith', 'M'],
    ['  Bob   Lee  ', 'bob', 'B'],
    ['élodie durand', 'edurand', 'É'],
    ['李 小龙', 'xli', '李'],
    ['(Sam) Park', 'spark', 'S'],
  ])('takes the first word’s initial from %j', (name, username, expected) => {
    expect(avatarInitials(name, username)).toBe(expected);
  });

  it('keeps a decomposed accent with its letter', () => {
    expect(avatarInitials('E\u0301lodie', 'e')).toBe('\u00c9');
  });

  /**
   * T-31: server accounts issued as `crew_alice`, `crew_bob`, … all start with
   * "c", so a rule that read only the start of a word gave every person in the
   * workspace the same grey "C". With no display name set, the display name IS
   * the username, so this is the path every new member takes.
   */
  it.each([
    ['crew_alice', 'A'],
    ['crew_bob', 'B'],
    ['crew_carol', 'C'],
    ['crew_dave', 'D'],
    ['lab.erin', 'E'],
    ['lab-frank', 'F'],
    ['crew__grace_', 'G'],
    ['alice_2', 'A'],
    ['_alice', 'A'],
  ])('reads a separated handle %j from its last part', (handle, expected) => {
    expect(avatarInitials(handle, handle)).toBe(expected);
  });

  /**
   * With no display name set, a member is shown by the name the daemon projects
   * for them: the username, with any `@` removed (`sanitize_display_name`). For
   * a fully qualified SSSD account that is `bobad.ucsf.edu`, whose last part is
   * the realm every member shares — so a rule that split the projected name read
   * "E" for all of them. The realm is never read.
   */
  it.each([
    [
      'crew_',
      ['crew_alice', 'crew_bob', 'crew_carol', 'crew_dave', 'crew_erin'],
      ['A', 'B', 'C', 'D', 'E'],
    ],
    [
      'SSSD',
      [
        'alice@ad.ucsf.edu',
        'bob@ad.ucsf.edu',
        'carol@ad.ucsf.edu',
        'crew_dave@ad.ucsf.edu',
        'frank@ad.ucsf.edu',
      ],
      ['A', 'B', 'C', 'D', 'F'],
    ],
  ])('tells the people of a %s workspace apart', (_shape, usernames, expected) => {
    const projected = usernames.map((username) => username.replace(/@/g, ''));
    const initials = usernames.map((username, i) => avatarInitials(projected[i], username));
    expect(initials).toEqual(expected);
    expect(new Set(initials).size).toBe(usernames.length);
    // A legacy daemon forwards the nickname as stored, which is the username itself.
    expect(usernames.map((username) => avatarInitials(username, username))).toEqual(expected);
  });

  it('reads an SSSD account by its account part, never its realm', () => {
    expect(avatarInitials('bobad.ucsf.edu', 'bob@ad.ucsf.edu')).toBe('B');
    expect(avatarInitials('BobAD.ucsf.edu', 'bob@ad.ucsf.edu')).toBe('B');
    expect(avatarInitials('crew_bobad.ucsf.edu', 'crew_bob@ad.ucsf.edu')).toBe('B');
    expect(avatarInitials(null, 'bob@ad.ucsf.edu')).toBe('B');
    expect(avatarInitials(null, 'crew_bob@ad.ucsf.edu')).toBe('B');
    expect(avatarInitials(null, 'alice.chen@ucsf.edu')).toBe('C');
    expect(avatarInitials('', 'bob\uFF20ad.ucsf.edu')).toBe('B');
    for (const username of ['bob@ad.ucsf.edu', 'alice@ad.ucsf.edu', 'alice.chen@ucsf.edu']) {
      expect(avatarInitials(username.replace('@', ''), username)).not.toBe('E');
      expect(avatarInitials(null, username)).not.toMatch(/^E/);
    }
  });

  it('reads a real display name by its first word, separators and all', () => {
    expect(avatarInitials('Alice Chen', 'crew_alice')).toBe('A');
    expect(avatarInitials('Carol Nguyen', 'crew_carol')).toBe('C');
    expect(avatarInitials('Mary-Jane Watson', 'mjw')).toBe('M');
    expect(avatarInitials('J.R.R. Tolkien', 'jrrt')).toBe('J');
    expect(avatarInitials('Bob Lee', 'bob@ad.ucsf.edu')).toBe('B');
    // The name the person chose, not their username, gives the letter.
    expect(avatarInitials('Zoe Adams', 'crew_alice')).toBe('Z');
    expect(avatarInitials('Henry Ito', 'hito@ad.ucsf.edu')).toBe('H');
  });

  /**
   * The last-part rule is for a name nobody chose. A one-word name a person
   * typed keeps its first letter, whatever joins its parts.
   */
  it.each([
    ['Jean-Luc', 'jpicard', 'J'],
    ['A.J.', 'ajones', 'A'],
    ['Mary-Jane', 'mjw', 'M'],
    ['st.john', 'sjohn', 'S'],
    ['crew_alice', 'alice', 'C'],
    ['bob', 'bob@ad.ucsf.edu', 'B'],
  ])('reads a chosen one-word name %j from its start', (name, username, expected) => {
    expect(avatarInitials(name, username)).toBe(expected);
  });

  it('falls back to the first letter of an unseparated username', () => {
    expect(avatarInitials('', 'bob')).toBe('B');
    expect(avatarInitials(null, 'bob')).toBe('B');
    expect(avatarInitials('🧬 🔬', 'alice')).toBe('A');
    expect(avatarInitials(undefined, '@x9')).toBe('X');
    expect(avatarInitials(undefined, '9lives')).toBe('9');
    // One lettered part is not a separated handle.
    expect(avatarInitials(null, 'crew_')).toBe('C');
  });

  it('reads a separated username the same way as a separated display name', () => {
    expect(avatarInitials(null, 'crew_bob')).toBe('B');
    expect(avatarInitials('🧬', 'crew_bob')).toBe('B');
    expect(avatarInitials('', 'lab.erin')).toBe('E');
    expect(avatarInitials(null, 'crew_bob')).toBe(avatarInitials('crew_bob', 'crew_bob'));
  });

  it('yields nothing rather than an ID-like placeholder when there is nothing to read', () => {
    expect(avatarInitials('', '')).toBe('');
    expect(avatarInitials(null, null)).toBe('');
    expect(avatarInitials('🧬', '___')).toBe('');
  });

  /**
   * Q3-62: the rule itself gives one character, so no caller — a tile of any
   * size, or anything that borrows the rule later — can show a person as "HI"
   * in one place and "H" in another.
   */
  it('gives one character for every shape of name', () => {
    const names: Array<[string | null | undefined, string | null]> = [
      ['Henry Ito', 'crew_henry'],
      ['Gina Rossi', 'crew_gina'],
      ['Mary Ann Smith', 'msmith'],
      ['李 小龙', 'xli'],
      ['E\u0301lodie Durand', 'ed'],
      ['crew_bob', 'crew_bob'],
      ['bobad.ucsf.edu', 'bob@ad.ucsf.edu'],
      [null, 'bob'],
      [null, 'bob@ad.ucsf.edu'],
      ['🧬 🔬', 'alice'],
      [undefined, '@x9'],
    ];
    for (const [name, username] of names) {
      // One letter or digit, with the marks that belong to it.
      expect(avatarInitials(name, username)).toMatch(/^[\p{L}\p{N}]\p{M}*$/u);
    }
  });

  it('stays one character when upper-casing spells the letter as two', () => {
    // U+FB01 is one letter that upper-cases to "FI".
    expect('\uFB01'.toLocaleUpperCase()).toBe('FI');
    expect(avatarInitials('\uFB01ona Smith', 'fsmith')).toBe('F');
  });
});

describe('Avatar', () => {
  it('shows the derived initial and is decorative by default', () => {
    const { container } = render(<Avatar name="Alice Chen" username="alice" />);
    expect(tile(container).textContent).toBe('A');
    expect(tile(container)).toHaveAttribute('aria-hidden', 'true');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('draws a member with no display name by the last part of their handle', () => {
    const { container, rerender } = render(<Avatar name="crew_alice" username="crew_alice" />);
    expect(tile(container)).toHaveTextContent(/^A$/);
    rerender(<Avatar name="bobad.ucsf.edu" username="bob@ad.ucsf.edu" />);
    expect(tile(container)).toHaveTextContent(/^B$/);
  });

  it('prefers the chosen avatar text, clamped to two characters', () => {
    const { container, rerender } = render(<Avatar fallback="🧑‍🔬" name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^🧑‍🔬$/u);
    rerender(<Avatar fallback="Alice" name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^Al$/);
    // A blank choice is no choice.
    rerender(<Avatar fallback="   " name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^A$/);
  });

  it('draws an icon in place of letters for an agent', () => {
    const { container } = render(
      <Avatar shape="square" icon={<Bot data-testid="bot" />} name="Alice Chen" />
    );
    expect(screen.getByTestId('bot')).toBeInTheDocument();
    expect(tile(container).textContent).toBe('');
  });

  it('is a named image when given a label', () => {
    render(<Avatar name="Bob Lee" label="Bob Lee (@bob)" />);
    expect(screen.getByRole('img', { name: 'Bob Lee (@bob)' })).toHaveTextContent(/^B$/);
  });

  it('defaults to a 32px circle and carries size, shape and ring as hooks', () => {
    const { container, rerender } = render(<Avatar name="Alice Chen" />);
    expect(tile(container)).toHaveAttribute('data-size', '32');
    expect(tile(container)).toHaveAttribute('data-shape', 'circle');
    expect(tile(container)).not.toHaveAttribute('data-ring');
    expect(tile(container)).toHaveClass('biorouter-avatar');

    for (const size of [20, 24, 32] as const) {
      rerender(<Avatar name="Alice Chen" size={size} shape="square" ring />);
      expect(tile(container)).toHaveAttribute('data-size', String(size));
      expect(tile(container)).toHaveAttribute('data-shape', 'square');
      expect(tile(container)).toHaveAttribute('data-ring', 'true');
    }
  });

  /**
   * jsdom does not load main.css, so the geometry and the skin are asserted at
   * the source: three sizes, a full radius for people and a ladder radius for
   * objects, and a ground/ink pair from tokens.
   */
  it('authors every size and shape in main.css from tokens', () => {
    for (const size of [20, 24, 32]) {
      expect(MAIN_CSS).toMatch(
        new RegExp(
          `\\.biorouter-avatar\\[data-size='${size}'\\] \\{\\s*width: ${size}px;\\s*height: ${size}px;`
        )
      );
    }
    expect(MAIN_CSS).toMatch(
      /\.biorouter-avatar\[data-shape='circle'\] \{\s*border-radius: var\(--radius-full\);/
    );
    expect(MAIN_CSS).toMatch(
      /\.biorouter-avatar\[data-shape='square'\] \{\s*border-radius: var\(--radius-/
    );
    const base = MAIN_CSS.slice(MAIN_CSS.indexOf('.biorouter-avatar {'));
    expect(base.slice(0, base.indexOf('}'))).toMatch(
      /background-color: var\(--background-medium\);[\s\S]*color: var\(--text-muted\);/
    );
  });
});

/**
 * Q3-62 (carol R3-10, gina F13, henry F7): the header's member stack draws a
 * person at 20px, member rows at 24px and messages at 32px. A 20px tile holds
 * one character (Q2-70: "CN" read as "Cɴ"), so the three showed Henry as "H",
 * "HI" and "HI" — one person, two identities side by side. A derived initial is
 * now one letter everywhere; only a pair the person chose shows in full, and
 * only at 28px and above.
 */
describe('Avatar — one person, one initial at every size (Q3-62)', () => {
  const SIZES = [20, 24, 32] as const;

  it('shows a chosen pair in full at 28px and above, and its first character below', () => {
    expect(AVATAR_PAIR_MIN_SIZE).toBe(28);
    expect(avatarTextLimit(16)).toBe(1);
    expect(avatarTextLimit(20)).toBe(1);
    expect(avatarTextLimit(24)).toBe(1);
    expect(avatarTextLimit(27)).toBe(1);
    expect(avatarTextLimit(28)).toBe(2);
    expect(avatarTextLimit(32)).toBe(2);
  });

  it('draws Henry as "H" and Gina as "G" in the header stack, member rows and messages', () => {
    for (const [name, username, initial] of [
      ['Henry Ito', 'crew_henry', 'H'],
      ['Gina Rossi', 'crew_gina', 'G'],
      ['Carol Nguyen', 'crew_carol', 'C'],
    ] as const) {
      const drawn = SIZES.map((size) => {
        const { container, unmount } = render(
          <Avatar size={size} name={name} username={username} />
        );
        const text = tile(container).textContent;
        unmount();
        return text;
      });
      expect(drawn).toEqual([initial, initial, initial]);
    }
  });

  it('draws the same one letter at every size for every shape of name', () => {
    for (const [name, username, expected] of [
      ['Alice Chen', 'crew_alice', 'A'],
      ['crew_bob', 'crew_bob', 'B'],
      [null, 'bob', 'B'],
      ['bobad.ucsf.edu', 'bob@ad.ucsf.edu', 'B'],
      ['élodie durand', 'edurand', 'É'],
      ['李 小龙', 'xli', '李'],
      ['E\u0301lodie Durand', 'ed', '\u00c9'],
      ['\uFB01ona Smith', 'fsmith', 'F'],
    ] as const) {
      for (const size of SIZES) {
        const { container, unmount } = render(
          <Avatar size={size} name={name} username={username} />
        );
        expect(tile(container).textContent).toBe(expected);
        unmount();
      }
    }
  });

  it('shows a chosen pair in full at 32px and its first character at 20 and 24', () => {
    const { container, rerender } = render(<Avatar size={20} fallback="CN" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('C');
    rerender(<Avatar size={24} fallback="CN" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('C');
    rerender(<Avatar size={32} fallback="CN" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('CN');
    // Still clamped to what the tile holds: a long choice cannot overflow it.
    rerender(<Avatar size={32} fallback="Carol" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('Ca');
  });

  it('keeps a chosen emoji whole at every size', () => {
    const { container, rerender } = render(
      <Avatar size={20} fallback="🧑‍🔬🧬" name="Carol Nguyen" />
    );
    expect(tile(container).textContent).toBe('🧑‍🔬');
    rerender(<Avatar size={24} fallback="🧑‍🔬🧬" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('🧑‍🔬');
    rerender(<Avatar size={32} fallback="🧑‍🔬🧬" name="Carol Nguyen" />);
    expect(tile(container).textContent).toBe('🧑‍🔬🧬');
    for (const size of SIZES) {
      rerender(<Avatar size={size} fallback="🧬" name="Carol Nguyen" />);
      expect(tile(container).textContent).toBe('🧬');
    }
  });

  it('still draws an agent’s glyph at 20px', () => {
    const { container } = render(
      <Avatar size={20} shape="square" icon={<Bot data-testid="bot" />} name="Alice Chen" />
    );
    expect(screen.getByTestId('bot')).toBeInTheDocument();
    expect(tile(container).textContent).toBe('');
  });
});

/**
 * D-AVATAR (carol F4): every tile was the same grey, so telling people apart
 * meant reading every name. A person wears one of eight hue pairs, picked from
 * the canonical `@username` — never the display name, which the person chooses
 * — and stable for every viewer on every device.
 */
describe('avatarHue — a person’s colour', () => {
  /**
   * Pinned: the hue is a pure function of the username, so these values are
   * what every device, and every later build, shows. Changing the function
   * recolours every person at once; if that is ever deliberate, change this
   * table with it.
   */
  it.each<[string, number]>([
    ['crew_alice', 7],
    ['crew_bob', 6],
    ['crew_carol', 7],
    ['crew_dave', 5],
    ['crew_erin', 3],
    ['crew_frank', 8],
    ['bob', 8],
    ['bob@ad.ucsf.edu', 8],
    ['李小龙', 7],
  ])('gives %j hue %i', (username, hue) => {
    expect(avatarHue(username)).toBe(hue);
  });

  it('is always one of the eight hues', () => {
    for (let i = 0; i < 2000; i++) {
      const hue = avatarHue(`member${i}`);
      expect(Number.isInteger(hue)).toBe(true);
      expect(hue).toBeGreaterThanOrEqual(1);
      expect(hue).toBeLessThanOrEqual(AVATAR_HUE_COUNT);
    }
  });

  it('spreads people across all eight hues', () => {
    const counts = new Array<number>(AVATAR_HUE_COUNT + 1).fill(0);
    const people = 8000;
    for (let i = 0; i < people; i++) counts[avatarHue(`member${i}`)!] += 1;
    for (let hue = 1; hue <= AVATAR_HUE_COUNT; hue++) {
      expect(counts[hue] / people).toBeGreaterThan(0.1);
      expect(counts[hue] / people).toBeLessThan(0.15);
    }
  });

  /**
   * `hash % 8` of an FNV hash reads only the low three bits of each byte, so
   * usernames whose letters differ by a multiple of eight (`a`, `i`, `q`, `y`)
   * would always share a colour. The fold lets every byte reach the hue.
   */
  it('lets every byte of the username reach the hue', () => {
    const hues = ['a', 'i', 'q', 'y'].map((letter) => avatarHue(`crew_${letter}lice`));
    expect(new Set(hues).size).toBeGreaterThan(1);
  });

  it('reads one account one way: case, a leading @, spaces and Unicode form aside', () => {
    const hue = avatarHue('crew_alice');
    for (const spelling of ['Crew_Alice', '@crew_alice', '\uFF20crew_alice', '  crew_alice ']) {
      expect(avatarHue(spelling)).toBe(hue);
    }
    expect(avatarHue('e\u0301lodie')).toBe(avatarHue('\u00e9lodie'));
    expect(avatarHue('Bob@AD.UCSF.EDU')).toBe(avatarHue('bob@ad.ucsf.edu'));
  });

  it('has no hue without a usable username', () => {
    for (const username of [undefined, null, '', '   ', '@', '\uFF20']) {
      expect(avatarHue(username)).toBeNull();
    }
  });
});

describe('Avatar hue', () => {
  const hueOf = (container: HTMLElement) => tile(container).getAttribute('data-hue');

  it('paints a person with the hue of their username, at every size', () => {
    for (const size of [20, 24, 32] as const) {
      const { container, unmount } = render(
        <Avatar size={size} name="Carol Nguyen" username="crew_carol" />
      );
      expect(hueOf(container)).toBe(String(avatarHue('crew_carol')));
      unmount();
    }
  });

  it('takes the hue from the username alone, never the display name or chosen avatar', () => {
    const { container, rerender } = render(<Avatar name="Carol Nguyen" username="crew_carol" />);
    const carol = hueOf(container);
    for (const [name, fallback] of [
      ['Alice Chen', null],
      ['crew_alice', null],
      ['Carol Nguyen', '🧬'],
      [null, 'AC'],
    ] as const) {
      rerender(<Avatar name={name} fallback={fallback} username="crew_carol" />);
      expect(hueOf(container)).toBe(carol);
    }
    // A display name copied from someone else does not bring their colour.
    const erin = String(avatarHue('crew_erin'));
    expect(erin).not.toBe(carol);
    rerender(<Avatar name="Carol Nguyen" username="crew_erin" />);
    expect(hueOf(container)).toBe(erin);
  });

  it('keeps the neutral tile for an agent, an object and a person not known yet', () => {
    const { container, rerender } = render(
      <Avatar shape="square" icon={<Bot />} username="crew_carol" />
    );
    expect(tile(container)).not.toHaveAttribute('data-hue');
    rerender(<Avatar shape="square" name="Analysis Lab" username="crew_carol" />);
    expect(tile(container)).not.toHaveAttribute('data-hue');
    rerender(<Avatar fallback="?" />);
    expect(tile(container)).not.toHaveAttribute('data-hue');
    rerender(<Avatar name="Carol Nguyen" />);
    expect(tile(container)).not.toHaveAttribute('data-hue');
  });

  /**
   * The component's count, the theme contract's and main.css's rules must be
   * one number: a ninth hue with no rule would paint nothing, and a rule the
   * component never names is dead. Every rule paints one token pair, the pair
   * `check-contrast.mjs` measures.
   */
  it('names exactly the hues the theme contract declares and main.css paints', () => {
    expect(THEME_CONTRACT).toMatch(
      new RegExp(`export const AVATAR_HUE_COUNT = ${AVATAR_HUE_COUNT};`)
    );
    for (let hue = 1; hue <= AVATAR_HUE_COUNT; hue++) {
      expect(MAIN_CSS).toMatch(
        new RegExp(
          `\\.biorouter-avatar\\[data-hue='${hue}'\\] \\{\\s*background-color: var\\(--avatar-hue-${hue}-bg\\);\\s*color: var\\(--avatar-hue-${hue}-fg\\);`
        )
      );
    }
    expect(MAIN_CSS).not.toMatch(
      new RegExp(`\\.biorouter-avatar\\[data-hue='${AVATAR_HUE_COUNT + 1}'\\]`)
    );
  });
});
