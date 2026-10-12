import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  ViewPrimaryParticipant,
  engageViewPrimary,
  useViewPrimaryVariant,
  viewPrimaryOwner,
} from './viewPrimary';

describe('viewPrimaryOwner (FR-R5-2)', () => {
  it('is the latest engaged participant that is present', () => {
    expect(viewPrimaryOwner(['llamacpp', 'ollama'], ['llamacpp', 'ollama'])).toBe('ollama');
    // A closed row, or one in another tab, is not present.
    expect(viewPrimaryOwner(['llamacpp', 'claude_code'], ['llamacpp'])).toBe('llamacpp');
  });

  it('falls to the first present participant when none was engaged, and to none when none is', () => {
    expect(viewPrimaryOwner(['llamacpp'], ['key box', 'codex'])).toBe('key box');
    expect(viewPrimaryOwner(['llamacpp'], [])).toBeNull();
  });
});

describe('engageViewPrimary', () => {
  it('moves the key to the end, once', () => {
    expect(engageViewPrimary(['a', 'b', 'c'], 'a')).toEqual(['b', 'c', 'a']);
    expect(engageViewPrimary(['a'], 'b')).toEqual(['a', 'b']);
  });

  it('returns the same array when the key is already the latest, so a focus re-renders nothing', () => {
    const engaged = ['a', 'b'];
    expect(engageViewPrimary(engaged, 'b')).toBe(engaged);
  });
});

function Primary({ ready }: { ready?: boolean }) {
  return <button className={useViewPrimaryVariant(ready)}>Use</button>;
}

describe('useViewPrimaryVariant', () => {
  it('keeps a lone panel’s primary: outside a catalog a panel is its own view', () => {
    render(<Primary />);
    expect(screen.getByRole('button')).toHaveClass('default');
  });

  it('is secondary while the panel’s own action is not ready', () => {
    render(<Primary ready={false} />);
    expect(screen.getByRole('button')).toHaveClass('secondary');
  });

  it('is secondary in a panel that does not hold its view’s primary', () => {
    render(
      <>
        <ViewPrimaryParticipant holds onEngage={() => {}}>
          <Primary />
        </ViewPrimaryParticipant>
        <ViewPrimaryParticipant holds={false} onEngage={() => {}}>
          <Primary />
        </ViewPrimaryParticipant>
      </>
    );
    const [held, other] = screen.getAllByRole('button');
    expect(held).toHaveClass('default');
    expect(other).toHaveClass('secondary');
  });

  it('engages a participant when focus moves into it, and draws no box of its own', () => {
    const onEngage = vi.fn();
    const { container } = render(
      <ViewPrimaryParticipant holds={false} onEngage={onEngage}>
        <Primary />
      </ViewPrimaryParticipant>
    );
    screen.getByRole('button').focus();
    expect(onEngage).toHaveBeenCalledTimes(1);
    expect((container.firstElementChild as HTMLElement).style.display).toBe('contents');
  });
});
