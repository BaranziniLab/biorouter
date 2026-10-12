import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The Settings band tabs' wiring, held at the source: jsdom computes no layout and applies no
 * stylesheet, so a component test that reads a tab's position sees nothing and passes whether the
 * rule exists or not (`styles/composerFocus.test.ts` makes the same argument for the composer).
 * The geometry itself is measured in Chromium by `settingsBand.browser.test.tsx`.
 *
 * What is Settings' own is one thing: the 16px margin that puts the title 24px from the first tab
 * (the bar's 8px gap plus this). That the tab text shares the title's centre line, and the
 * underline lands on the hairline, is the shared band's rule in `main.css`
 * (`.biorouter-page-header-tabs`, WS-VIEWS-A), which reaches Settings only because `SettingsView`
 * hands its list to `PageHeader`'s `tabs` slot, where the list is the wrapper's direct child. The
 * per-view copies of that rule that used to sit in `settings.css` are gone, so these pins name
 * the dependency instead.
 */
const css = readFileSync(join(__dirname, 'settings.css'), 'utf8');
const view = readFileSync(join(__dirname, 'SettingsView.tsx'), 'utf8');
const main = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

/** The declarations of the first rule in `source` whose selector is `selector`, or null. */
function rule(source: string, selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\>]/g, '\\$&');
  const match = source.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  return match ? match[1] : null;
}

describe('the Settings band tabs', () => {
  it('give the title room: the first tab starts 16px beyond the bar’s gap', () => {
    const list = rule(css, '.biorouter-settings-tabs');
    expect(list).not.toBeNull();
    expect(list).toMatch(/margin-inline-start:\s*16px/);
  });

  it('do not restate the shared band rule: one rule decides where the label sits', () => {
    // Two copies drift apart, and the second decides nothing until the first changes.
    expect(css).not.toMatch(/\.biorouter-settings-tabs\s*>\s*\[role=['"]tab['"]\]/);
    expect(rule(css, '.biorouter-settings-tabs')).not.toMatch(/align-self|height/);
  });

  it('rely on the shared band rule, which is still there', () => {
    const wrapper = rule(main, '.biorouter-page-header-tabs');
    expect(wrapper).not.toBeNull();
    expect(wrapper).toMatch(/align-items:\s*stretch/);
    const trigger = rule(main, ".biorouter-page-header-tabs > [role='tablist'] > [role='tab']");
    expect(trigger).not.toBeNull();
    expect(trigger).toMatch(/align-self:\s*stretch/);
    expect(trigger).toMatch(/height:\s*auto/);
  });

  it('are wired: the view imports the stylesheet and puts the class on a list in the tabs slot', () => {
    expect(view).toMatch(/import\s+'\.\/settings\.css'/);
    expect(view).toMatch(
      /<PageHeader[\s\S]*?\btabs=\{[\s\S]*?<TabsList[\s\S]*?className="biorouter-settings-tabs\b/
    );
  });
});
