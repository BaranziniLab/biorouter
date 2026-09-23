import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { Button } from './button';
import { Input } from './input';

afterEach(cleanup);

describe('shared controls follow the font-size spacing preference', () => {
  it.each([
    ['xs', 'compact'],
    ['sm', 'sm'],
    ['default', 'md'],
    ['lg', 'lg'],
  ] as const)('keeps %s icon buttons square using one scalable size token', (size, token) => {
    render(<Button size={size} shape="round" aria-label="Action" />);
    expect(screen.getByRole('button', { name: 'Action' })).toHaveClass(
      `h-control-${token}`,
      `w-control-${token}`
    );
  });

  it('uses the same scalable default height for buttons and inputs', () => {
    render(
      <>
        <Button>Save</Button>
        <Input aria-label="Name" />
      </>
    );
    expect(screen.getByRole('button', { name: 'Save' })).toHaveClass('h-control-md');
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveClass('h-control-md');
  });

  it('still lets callers request an unboxed button', () => {
    render(<Button className="h-auto p-0">Inline action</Button>);
    expect(screen.getByRole('button')).toHaveClass('h-auto');
    expect(screen.getByRole('button')).not.toHaveClass('h-control-md');
  });
});
