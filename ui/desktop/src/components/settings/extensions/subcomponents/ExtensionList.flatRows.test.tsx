import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { FixedExtensionEntry } from '../../../ConfigContext';
import ExtensionList from './ExtensionList';

const entry = (name: string, enabled: boolean) =>
  ({
    type: 'stdio',
    name,
    description: `${name} description`,
    cmd: 'node',
    args: ['server.js'],
    enabled,
  }) as FixedExtensionEntry;

/**
 * R-27: the Extensions rows are told apart by space and the hover wash, not by a line under each,
 * and the way a list says so is the one shared class, `.br-rows-flat` (main.css, rows block),
 * which also gives the line back under forced colours. `extensionRowGeometry.browser.test.tsx`
 * measures what the class does, but it builds its list shell by hand, so only this fails when the
 * component stops putting the class on the shells it renders.
 */
describe('ExtensionList: the group shells opt out of the row hairline', () => {
  it('puts the shared flat-rows class on every group shell, and nowhere else', () => {
    const { container } = render(
      <ExtensionList
        extensions={[entry('alpha', true), entry('bravo', false)]}
        onToggle={async () => true}
      />
    );

    const shells = [...container.querySelectorAll('.biorouter-list-shell')];
    // One shell for Enabled, one for Available.
    expect(shells).toHaveLength(2);
    for (const shell of shells) expect(shell).toHaveClass('br-rows-flat');
    expect(container.querySelectorAll('.br-rows-flat')).toHaveLength(shells.length);
  });
});
