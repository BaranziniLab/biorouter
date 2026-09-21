import { describe, expect, it } from 'vitest';
import postcss from 'postcss';
import { fontScale } from '../../vite-plugins/fontScale.mjs';

const transform = async (css: string) =>
  (await postcss([fontScale()]).process(css, { from: undefined })).css;

describe('typography scaling in compiled CSS', () => {
  it('scales shorthand sizes and leading while preserving relative math fonts', async () => {
    const css = await transform(`.row { font: 12px/20px var(--font-mono); }
      .label { font: bold 14px Arial; }
      .relative-leading { font: 14px/1.5 Arial; }
      .math { font: normal 1.21em KaTeX_Main; }`);
    expect(css).toContain(
      'font: calc(12px * var(--app-font-scale, 1.07))/calc(20px * var(--app-font-scale, 1.07)) var(--font-mono)'
    );
    expect(css).toContain('font: bold calc(14px * var(--app-font-scale, 1.07)) Arial');
    expect(css).toContain('font: normal 1.21em KaTeX_Main');
    expect(css).toContain('font: calc(14px * var(--app-font-scale, 1.07))/1.5 Arial');
    expect(await transform(css)).toBe(css);
  });

  it('scales Tailwind spacing-based leading without changing layout spacing', async () => {
    const css = await transform(`.leading-3 {
      --tw-leading: calc(var(--spacing) * 3);
      line-height: calc(var(--spacing) * 3);
      padding: calc(var(--spacing) * 3);
    }
    .label { line-height: var(--tw-leading, var(--text-label--line-height)); }`);
    expect(css).toContain(
      '--tw-leading: calc(calc(var(--spacing) * 3) * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain(
      'line-height: calc(calc(var(--spacing) * 3) * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain('padding: calc(var(--spacing) * 3)');
    expect(css).toContain('line-height: var(--tw-leading, var(--text-label--line-height))');
    expect(await transform(css)).toBe(css);
  });
  it('scales semantic tokens and hardcoded pixel/rem text exactly once', async () => {
    const css =
      await transform(`:root { --text-label: 14px; --text-label--line-height: 20px; --text-default: #333; }
      .label { font-size: var(--text-label); line-height: var(--text-label--line-height); }
      .px { font-size: 11px; line-height: 16px; padding: 11px; }
      .rem { font-size: .875rem; }
      .nested { font-size: 1.2em; line-height: 1.5; }`);
    expect(css).toContain('--text-label: calc(14px * var(--app-font-scale, 1.07))');
    expect(css).toContain('--text-label--line-height: calc(20px * var(--app-font-scale, 1.07))');
    expect(css).toContain('font-size: var(--text-label)');
    expect(css).toContain('font-size: calc(11px * var(--app-font-scale, 1.07))');
    expect(css).toContain('font-size: calc(.875rem * var(--app-font-scale, 1.07))');
    expect(css).toContain('padding: 11px');
    expect(css).toContain('--text-default: #333');
    expect(css).toContain('font-size: 1.2em; line-height: 1.5');
    expect(await transform(css)).toBe(css);
  });
});
