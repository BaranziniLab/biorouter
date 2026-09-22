import { describe, expect, it } from 'vitest';
import postcss from 'postcss';
import { fontScale } from '../../vite-plugins/fontScale.mjs';

const transform = async (css: string) =>
  (await postcss([fontScale()]).process(css, { from: undefined })).css;

describe('typography scaling in compiled CSS', () => {
  it('scales positive breathing room while preserving relative space and pane geometry', async () => {
    const css = await transform(`.content {
      padding: 8px 1rem;
      margin: 12px auto -4px 0;
      margin-top: calc(var(--spacing) * 2);
      gap: 0.5rem 1em;
      width: 400px;
      height: 100vh;
      top: 12px;
      border-width: 1px;
      transform: translateX(8px);
      margin-bottom: calc(100vh - 20px);
    }`);
    expect(css).toContain(
      'padding: calc(8px * var(--app-font-scale, 1.07)) calc(1rem * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain('margin: calc(12px * var(--app-font-scale, 1.07)) auto -4px 0');
    expect(css).toContain(
      'margin-top: calc(calc(var(--spacing) * 2) * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain('gap: calc(0.5rem * var(--app-font-scale, 1.07)) 1em');
    for (const unchanged of [
      'width: 400px',
      'height: 100vh',
      'top: 12px',
      'border-width: 1px',
      'transform: translateX(8px)',
      'margin-bottom: calc(100vh - 20px)',
    ])
      expect(css).toContain(unchanged);
    expect(await transform(css)).toBe(css);
  });

  it('scales shared control tokens once without growing app chrome or panel measures', async () => {
    const css = await transform(`:root {
      --control-md: 32px; --row-height: 40px; --row-height-rail: 32px;
      --md-code-pad: 14px 16px;
      --spacing: .25rem; --chrome-height: 44px; --dock-height: 36px;
      --tab-height: 32px; --measure-chat: 760px;
    }
    .control { height: var(--control-md); padding: var(--md-code-pad); }`);
    expect(css).toContain('--control-md: calc(32px * var(--app-font-scale, 1.07))');
    expect(css).toContain('--row-height: calc(40px * var(--app-font-scale, 1.07))');
    expect(css).toContain(
      '--md-code-pad: calc(14px * var(--app-font-scale, 1.07)) calc(16px * var(--app-font-scale, 1.07))'
    );
    for (const unchanged of [
      '--spacing: .25rem',
      '--chrome-height: 44px',
      '--dock-height: 36px',
      '--tab-height: 32px',
      '--measure-chat: 760px',
      'height: var(--control-md)',
      'padding: var(--md-code-pad)',
    ])
      expect(css).toContain(unchanged);
    expect(await transform(css)).toBe(css);
  });

  it('scales Tailwind space-y siblings and keeps intrinsic math and terminal geometry', async () => {
    const css = await transform(`.space-y-2 > * {
      margin-block-start: calc(calc(var(--spacing) * 2) * var(--tw-space-y-reverse));
      margin-block-end: calc(calc(var(--spacing) * 2) * calc(1 - var(--tw-space-y-reverse)));
    }
    .katex .vlist { margin: 2px; padding: 1rem; }
    .xterm-screen { padding: 8px; }
    .monaco-editor { margin: 2px; }`);
    expect(css).toContain(
      'margin-block-start: calc(calc(calc(var(--spacing) * 2) * var(--tw-space-y-reverse)) * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain(
      'margin-block-end: calc(calc(calc(var(--spacing) * 2) * calc(1 - var(--tw-space-y-reverse))) * var(--app-font-scale, 1.07))'
    );
    expect(css).toContain('.katex .vlist { margin: 2px; padding: 1rem; }');
    expect(css).toContain('.xterm-screen { padding: 8px; }');
    expect(css).toContain('.monaco-editor { margin: 2px; }');
    expect(await transform(css)).toBe(css);
  });
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

  it('scales Tailwind leading and padding together', async () => {
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
    expect(css).toContain('padding: calc(calc(var(--spacing) * 3) * var(--app-font-scale, 1.07))');
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
    expect(css).toContain('padding: calc(11px * var(--app-font-scale, 1.07))');
    expect(css).toContain('--text-default: #333');
    expect(css).toContain('font-size: 1.2em; line-height: 1.5');
    expect(await transform(css)).toBe(css);
  });
});
