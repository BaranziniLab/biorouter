/**
 * The code face: the installed SF Mono, registered as "SF Mono" (owner, 2026-11-12).
 *
 * The owner asked that code read like the text around it, the way Codex's does, while
 * still reading as code. Codex's code stack, which `--font-mono` now uses verbatim, is
 * `ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono",
 * monospace`. Under WebKit the first name is SF Mono, Apple's system monospace, drawn to
 * sit beside the system text. Under Chromium, measured on macOS 26 with
 * `CSS.getPlatformFontsForNode`, none of the first three names resolves, so every code
 * surface (markdown code, tool arguments and output, the artifact panel, the terminal)
 * fell back to Menlo: wider, heavier and visibly unlike the UI text.
 *
 * The desktop main process hands over the installed font file's bytes
 * (`get-system-mono-font`, macOS only) and this registers them with the FontFace API
 * under the family the stack already names, so `"SF Mono"` resolves to the system face.
 * Bytes registered this way are not a fetch, so the page's `font-src` policy is untouched.
 * Windows, Linux and a `biorouter serve` browser get null and keep Codex's stack as it
 * stands (Consolas, Liberation Mono; Safari reaches SF Mono through `ui-monospace`).
 *
 * The file is a variable font with a weight axis, so the face is declared over the whole
 * range and a 400 or 600 in CSS lands on its own instance rather than a synthetic bold.
 */
export const SYSTEM_MONO_FAMILY = 'SF Mono';

let loading: Promise<boolean> | null = null;
let ready = false;

/** True once the face is registered: a surface that measured glyphs before then re-measures. */
export function isSystemMonoFontReady(): boolean {
  return ready;
}

/** Registers the system mono face once; resolves `true` when it is ready to draw. */
export function loadSystemMonoFont(): Promise<boolean> {
  if (loading) return loading;
  loading = (async () => {
    try {
      const font = await window.electron?.getSystemMonoFont?.();
      if (!font || typeof FontFace === 'undefined' || !document.fonts) return false;
      // A copy into a plain ArrayBuffer: IPC hands over a view whose buffer may be shared.
      const bytes = (view: Uint8Array) => new Uint8Array(view);
      const faces = [
        new FontFace(SYSTEM_MONO_FAMILY, bytes(font.regular), {
          weight: '100 900',
          style: 'normal',
        }),
      ];
      if (font.italic) {
        faces.push(
          new FontFace(SYSTEM_MONO_FAMILY, bytes(font.italic), {
            weight: '100 900',
            style: 'italic',
          })
        );
      }
      await Promise.all(faces.map((face) => face.load()));
      for (const face of faces) document.fonts.add(face);
      ready = true;
      return true;
    } catch {
      // A face that fails to load leaves the stack to fall through, exactly as before.
      return false;
    }
  })();
  return loading;
}

/** Test seam: forget the registration so a test can load again. */
export function resetSystemMonoFontForTest(): void {
  loading = null;
  ready = false;
}
