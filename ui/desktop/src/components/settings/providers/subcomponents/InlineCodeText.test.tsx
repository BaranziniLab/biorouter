import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { InlineCodeText } from './InlineCodeText';

describe('InlineCodeText', () => {
  it('draws a backticked command as inline code and drops the marks', () => {
    const { container } = render(
      <p>
        <InlineCodeText text="Uses your own installed `claude` command-line app." />
      </p>
    );
    const code = container.querySelector('code');
    expect(code).toHaveTextContent('claude');
    expect(code).toHaveClass('font-mono');
    expect(container.textContent).toBe('Uses your own installed claude command-line app.');
  });

  it('handles several pairs, and a pair at either end', () => {
    const { container } = render(
      <p>
        <InlineCodeText text="`codex` or `claude`" />
      </p>
    );
    expect([...container.querySelectorAll('code')].map((node) => node.textContent)).toEqual([
      'codex',
      'claude',
    ]);
    expect(container.textContent).toBe('codex or claude');
  });

  it('leaves a stray backtick, and text with none, exactly as it was', () => {
    const { container } = render(
      <p>
        <InlineCodeText text="a ` b" />
        <InlineCodeText text=" plain" />
      </p>
    );
    expect(container.querySelector('code')).toBeNull();
    expect(container.textContent).toBe('a ` b plain');
  });

  it('never turns the text into markup', () => {
    const { container } = render(
      <p>
        <InlineCodeText text="`<b>x</b>`" />
      </p>
    );
    expect(container.querySelector('b')).toBeNull();
    expect(container.querySelector('code')?.textContent).toBe('<b>x</b>');
  });
});
