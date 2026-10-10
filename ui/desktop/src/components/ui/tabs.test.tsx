import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { Tabs, TabsContent, TabsList, TabsTrigger } from './tabs';

const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
function body(selector: string): string {
  const at = CSS.indexOf(`${selector} {`);
  expect(at, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
  return CSS.slice(CSS.indexOf('{', at) + 1, CSS.indexOf('}', at));
}

function Strip() {
  return (
    <Tabs defaultValue="general">
      <TabsList aria-label="Sections">
        <TabsTrigger value="general">General</TabsTrigger>
        <TabsTrigger value="models">Models</TabsTrigger>
      </TabsList>
      <TabsContent value="general">General body</TabsContent>
      <TabsContent value="models">Models body</TabsContent>
    </Tabs>
  );
}

/**
 * Spec 2.6: the underline becomes ONE indicator that travels (`translate` plus `width`, 175ms),
 * and its first placement does not animate. jsdom measures every box as 0, so here the list
 * stays unplaced and each trigger's own bar is the underline; the travel itself is measured in
 * `controlsGeometry.browser.test.tsx`.
 */
describe('Tabs', () => {
  it('keeps the triggers as the tab stops and the indicator out of the accessibility tree', async () => {
    const user = userEvent.setup();
    const { container } = render(<Strip />);
    const indicator = container.querySelector('.br-tabs-indicator');
    expect(indicator).toHaveAttribute('aria-hidden');
    expect(screen.getAllByRole('tab')).toHaveLength(2);

    await user.click(screen.getByRole('tab', { name: 'Models' }));
    expect(screen.getByRole('tab', { name: 'Models' })).toHaveAttribute('data-state', 'active');
    expect(screen.getByText('Models body')).toBeVisible();
  });

  it('leaves the trigger bars as the underline until a real measurement exists', () => {
    const { container } = render(<Strip />);
    const list = screen.getByRole('tablist');
    expect(list).toHaveAttribute('data-placed', 'false');
    const indicator = container.querySelector('.br-tabs-indicator');
    expect(indicator).toHaveAttribute('data-placed', 'false');
    // The first placement lands at rest.
    expect(indicator).toHaveAttribute('data-motion-still');
    expect(screen.getByRole('tab', { name: 'General' })).toHaveClass(
      'data-[state=active]:after:bg-accent-bar'
    );
  });

  it('travels on translate and width over the tokens, and firms on keyboard focus', () => {
    const indicator = body('.br-tabs-indicator');
    expect(indicator).toMatch(/translate var\(--dur-fast-max\) var\(--ease-out\)/);
    expect(indicator).toMatch(/width var\(--dur-fast-max\) var\(--ease-out\)/);
    expect(indicator).toMatch(/background-color:\s*var\(--accent-bar\)/);
    expect(body('.br-tabs-indicator[data-motion-still]')).toMatch(/transition:\s*none/);
    expect(body(".br-tabs-list[data-placed='true'] > [role='tab']::after")).toMatch(
      /background-color:\s*transparent/
    );
    expect(body(".br-tabs-list:has(> [role='tab']:focus-visible) > .br-tabs-indicator")).toMatch(
      /block-size:\s*3px/
    );
  });

  it('opts the indicator into the resize rule, so it never glides against a drag', () => {
    const { container } = render(<Strip />);
    expect(container.querySelector('.br-tabs-indicator')).toHaveAttribute('data-motion-layout');
  });

  it('fades content in on the tokens, with no retired duration utility', () => {
    render(<Strip />);
    const panel = screen.getByRole('tabpanel');
    expect(panel).toHaveClass('br-tabs-content');
    expect(panel.className).not.toMatch(/duration-|motion-base/);
    expect(body('.br-tabs-content')).toMatch(/--tw-duration:\s*var\(--dur-fast-max\)/);
  });
});
