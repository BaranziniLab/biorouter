import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Bot } from '../icons/app-icons';
import { Avatar, avatarInitials } from './avatar';

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

const tile = (container: HTMLElement) =>
  container.querySelector('[data-slot="avatar"]') as HTMLElement;

describe('avatarInitials — the one fallback rule (L16)', () => {
  it.each([
    ['Alice Chen', 'alice', 'AC'],
    ['alice', 'alice', 'A'],
    ['Mary Ann Smith', 'msmith', 'MA'],
    ['  Bob   Lee  ', 'bob', 'BL'],
    ['élodie durand', 'edurand', 'ÉD'],
    ['李 小龙', 'xli', '李小'],
    ['(Sam) Park', 'spark', 'SP'],
  ])('takes up to two word initials from %j', (name, username, expected) => {
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

  it('tells the people of a crew_ workspace apart', () => {
    const people = ['crew_alice', 'crew_bob', 'crew_carol', 'crew_dave', 'crew_erin'];
    const initials = people.map((handle) => avatarInitials(handle, handle));
    expect(initials).toEqual(['A', 'B', 'C', 'D', 'E']);
    expect(new Set(initials).size).toBe(people.length);
  });

  it('still gives a real display name two initials, separators and all', () => {
    expect(avatarInitials('Alice Chen', 'crew_alice')).toBe('AC');
    expect(avatarInitials('Carol Nguyen', 'crew_carol')).toBe('CN');
    expect(avatarInitials('Mary-Jane Watson', 'mjw')).toBe('MW');
    expect(avatarInitials('J.R.R. Tolkien', 'jrrt')).toBe('JT');
  });

  it('falls back to the first two letters of an unseparated username', () => {
    expect(avatarInitials('', 'bob')).toBe('BO');
    expect(avatarInitials(null, 'bob')).toBe('BO');
    expect(avatarInitials('🧬 🔬', 'alice')).toBe('AL');
    expect(avatarInitials(undefined, '@x9')).toBe('X9');
    // One lettered part is not a separated handle.
    expect(avatarInitials(null, 'crew_')).toBe('CR');
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
  });
});

describe('Avatar', () => {
  it('shows derived initials and is decorative by default', () => {
    const { container } = render(<Avatar name="Alice Chen" username="alice" />);
    expect(tile(container)).toHaveTextContent('AC');
    expect(tile(container)).toHaveAttribute('aria-hidden', 'true');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('draws a member with no display name by the last part of their handle', () => {
    const { container } = render(<Avatar name="crew_alice" username="crew_alice" />);
    expect(tile(container)).toHaveTextContent(/^A$/);
  });

  it('prefers the chosen avatar text, clamped to two characters', () => {
    const { container, rerender } = render(<Avatar fallback="🧑‍🔬" name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^🧑‍🔬$/u);
    rerender(<Avatar fallback="Alice" name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^Al$/);
    // A blank choice is no choice.
    rerender(<Avatar fallback="   " name="Alice Chen" />);
    expect(tile(container)).toHaveTextContent(/^AC$/);
  });

  it('draws an icon in place of letters for an agent', () => {
    const { container } = render(
      <Avatar shape="square" icon={<Bot data-testid="bot" />} name="Alice Chen" />
    );
    expect(screen.getByTestId('bot')).toBeInTheDocument();
    expect(tile(container)).not.toHaveTextContent('AC');
  });

  it('is a named image when given a label', () => {
    render(<Avatar name="Bob Lee" label="Bob Lee (@bob)" />);
    expect(screen.getByRole('img', { name: 'Bob Lee (@bob)' })).toHaveTextContent('BL');
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
