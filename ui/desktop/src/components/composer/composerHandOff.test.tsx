import { StrictMode, useState } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import {
  composerHandOffSizeForTests,
  readComposerHandOff,
  useComposerHandOff,
} from './composerHandOff';

/**
 * V2-R8-3. A composer's hand-over to the composer that replaces it: read in the
 * replacement's first render, gone with the composer that published it.
 */
function Probe({
  composerKey,
  value,
  testId,
}: {
  composerKey?: string;
  value: string | null;
  testId: string;
}) {
  // What this mount was handed when it began, as a state initialiser reads it.
  const [received] = useState(() => readComposerHandOff<string>(composerKey, 'probe'));
  useComposerHandOff(composerKey, 'probe', value);
  return <span data-testid={testId}>{received ?? 'nothing'}</span>;
}

/** One probe, drawn in one of two places, as `BaseChat` draws its composer. */
function Seat({
  inCentre,
  composerKey,
  value,
}: {
  inCentre: boolean;
  composerKey?: string;
  value: string | null;
}) {
  const probe = <Probe composerKey={composerKey} value={value} testId="probe" />;
  return (
    <div>
      {inCentre ? <section>{probe}</section> : <p />}
      {!inCentre && <footer>{probe}</footer>}
    </div>
  );
}

describe('composerHandOff', () => {
  it('hands the value to the composer that replaces it in the same commit', () => {
    const view = render(<Seat inCentre composerKey="tab-1" value="settled" />);
    expect(screen.getByTestId('probe')).toHaveTextContent('nothing');
    const before = screen.getByTestId('probe');

    view.rerender(<Seat inCentre={false} composerKey="tab-1" value={null} />);
    // A new mount (the instrument), which began with what the old one stated.
    expect(screen.getByTestId('probe')).not.toBe(before);
    expect(screen.getByTestId('probe')).toHaveTextContent('settled');
    // The replacement publishes nothing (`null`), and the old entry left with
    // its composer.
    expect(composerHandOffSizeForTests()).toBe(0);
    view.unmount();
  });

  it('keeps nothing for a composer that mounts after the publisher is gone', () => {
    const first = render(<Seat inCentre composerKey="tab-1" value="settled" />);
    expect(composerHandOffSizeForTests()).toBe(1);
    first.unmount();
    expect(composerHandOffSizeForTests()).toBe(0);

    render(<Seat inCentre composerKey="tab-1" value={null} />);
    expect(screen.getByTestId('probe')).toHaveTextContent('nothing');
  });

  it('hands nothing across identities, and nothing without one', () => {
    const view = render(<Seat inCentre composerKey="tab-1" value="settled" />);
    view.rerender(<Seat inCentre={false} composerKey="tab-2" value={null} />);
    expect(screen.getByTestId('probe')).toHaveTextContent('nothing');
    view.unmount();

    const anonymous = render(<Seat inCentre value="settled" />);
    expect(composerHandOffSizeForTests()).toBe(0);
    anonymous.rerender(<Seat inCentre={false} value={null} />);
    expect(screen.getByTestId('probe')).toHaveTextContent('nothing');
  });

  it('keeps the newest value, and the replacement’s own entry survives the old cleanup', () => {
    const view = render(<Seat inCentre composerKey="tab-1" value="first" />);
    view.rerender(<Seat inCentre composerKey="tab-1" value="second" />);
    expect(readComposerHandOff('tab-1', 'probe')).toBe('second');

    // The replacement publishes at once; the old composer's cleanup must not
    // take the new entry with it.
    view.rerender(<Seat inCentre={false} composerKey="tab-1" value="third" />);
    expect(screen.getByTestId('probe')).toHaveTextContent('second');
    expect(readComposerHandOff('tab-1', 'probe')).toBe('third');
    view.unmount();
    expect(composerHandOffSizeForTests()).toBe(0);
  });

  it('survives StrictMode’s second run of every effect', () => {
    const view = render(
      <StrictMode>
        <Seat inCentre composerKey="tab-1" value="settled" />
      </StrictMode>
    );
    expect(readComposerHandOff('tab-1', 'probe')).toBe('settled');
    view.unmount();
    expect(composerHandOffSizeForTests()).toBe(0);
  });
});
