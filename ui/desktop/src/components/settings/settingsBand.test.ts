import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The Settings band's tab geometry, held at the source: jsdom computes no layout and applies no
 * stylesheet, so a component test that reads a tab's position sees nothing and passes whether the
 * rule exists or not (`styles/composerFocus.test.ts` makes the same argument for the composer).
 *
 * What the rules do, measured in the running layout (Chromium, 1440 wide): the band centres its
 * 14px title in the bar, and without these rules the tab wrapper bottom-aligned a 36px trigger,
 * so the tab text sat 3px under the title's line and 9px from it. With them the triggers fill the
 * band's height, the text shares the title's centre line, and the underline (positioned from the
 * list's bottom edge) still lands on the hairline.
 */
const css = readFileSync(join(__dirname, 'settings.css'), 'utf8');
const view = readFileSync(join(__dirname, 'SettingsView.tsx'), 'utf8');

/** The declarations of the first rule whose selector is `selector`, or null. */
function rule(selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\>]/g, '\\$&');
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  return match ? match[1] : null;
}

describe('the Settings band tabs', () => {
  it('stretch to the band and give the title room', () => {
    const list = rule('.biorouter-settings-tabs');
    expect(list).not.toBeNull();
    expect(list).toMatch(/align-self:\s*stretch/);
    expect(list).toMatch(/margin-inline-start:\s*16px/);
  });

  it('fill the list, so the tab text shares the title’s centre line', () => {
    const trigger = rule(".biorouter-settings-tabs > [role='tab']");
    expect(trigger).not.toBeNull();
    expect(trigger).toMatch(/height:\s*100%/);
  });

  it('are wired: the view imports the stylesheet and puts the class on its tab list', () => {
    expect(view).toMatch(/import\s+'\.\/settings\.css'/);
    expect(view).toMatch(/className="biorouter-settings-tabs\b/);
  });
});
