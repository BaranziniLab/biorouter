import { execFile } from 'node:child_process';
import os from 'node:os';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';
import log from 'electron-log';
import {
  IMAGE_MAX_DIMENSION,
  IMAGE_MAX_PIXELS,
  readFileHandleBounded,
} from './artifactPreviewLimits';

const run = promisify(execFile);

/**
 * HEIC previews, via the operating system's own decoder.
 *
 * **Why the OS and not a bundled library.** Every practical HEIC decoder is
 * libheif underneath, which is LGPL — manageable, but the harder problem is
 * that HEVC patents attach to the *technique*, not the implementation, so no
 * library choice removes them. Access Advance's own FAQ answers "software that
 * is free for users to download" with *"In general, HEVC software downloaded by
 * users requires a license."* The market agrees: `sharp` excludes HEIC from its
 * prebuilts and calls it patent-encumbered, `wasm-vips` compiles libheif with
 * the HEVC decoder switched off, and Debian ships it as a plugin that is not
 * installed by default.
 *
 * Decoding through a decoder the OS vendor has already licensed is the one
 * route that plausibly sidesteps that, and since HEIC files are overwhelmingly
 * iPhone and Mac artefacts it covers most real traffic for nothing — no bundle,
 * no copyleft, no build change.
 *
 * Elsewhere the panel says plainly that it cannot decode the format
 * (`describeUnsupportedFormat`), which is better than a half-answer. Shipping a
 * bundled decoder for those platforms is a real option, and one that needs a
 * licence decision rather than a commit.
 */

/** `sips` ships with macOS itself, not with Xcode. */
const SIPS = '/usr/bin/sips';

let supportsHeic: boolean | null = null;

/**
 * Whether this machine can convert HEIC.
 *
 * Probed once, at runtime, rather than assumed from the platform: the published
 * `sips` man page predates High Sierra and does not list HEIC among its formats
 * even though ImageIO has read it since 10.13, so the documentation is not a
 * reliable guide and the runtime answer is.
 */
export async function canConvertHeic(): Promise<boolean> {
  if (supportsHeic !== null) return supportsHeic;
  if (process.platform !== 'darwin') {
    supportsHeic = false;
    return false;
  }
  try {
    await fs.access(SIPS);
    supportsHeic = true;
  } catch {
    supportsHeic = false;
  }
  return supportsHeic;
}

/**
 * Largest PNG the converter will hand back.
 *
 * The bound below already keeps the raster inside the previewer's pixel budget,
 * so this is the backstop for a *dense* in-budget image: 32 megapixels of noise
 * can still deflate to well over 100 MB, and that buffer would be cloned across
 * the IPC boundary into the renderer. 64 MiB is the same ceiling the Office
 * previewer accepts for expanded content, and it clears a full-frame iPhone
 * photo (~20-30 MB as PNG) with room to spare.
 */
export const HEIC_PNG_MAX_BYTES = 64 * 1024 * 1024;

/**
 * Beyond this the source is not a photograph.
 *
 * The largest phone and medium-format sensors in production are around 200
 * megapixels, so a HEIC claiming more is a decompression bomb — a ~2 MB file
 * declaring 30000x30000 asks the decoder for ~3.6 GB. Refusing on the declared
 * extent, before the converter is spawned at all, keeps that decode out of
 * *every* process rather than merely out of ours.
 */
const HEIC_MAX_SOURCE_PIXELS = 200_000_000;

/**
 * The declared extent of a HEIC, read from its metadata.
 *
 * `sips -g` reads image properties through ImageIO; it does not decode pixels,
 * which is the whole reason this can be asked before committing to a
 * conversion.
 */
async function heicDimensions(filePath: string): Promise<{ width: number; height: number } | null> {
  try {
    const { stdout } = await run(SIPS, ['-g', 'pixelWidth', '-g', 'pixelHeight', filePath], {
      timeout: 20_000,
      windowsHide: true,
    });
    const width = Number(stdout.match(/pixelWidth:\s*(\d+)/)?.[1]);
    const height = Number(stdout.match(/pixelHeight:\s*(\d+)/)?.[1]);
    if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height)) return null;
    if (width <= 0 || height <= 0) return null;
    return { width, height };
  } catch (error) {
    log.info('[heic] could not read dimensions:', error);
    return null;
  }
}

/**
 * The `-Z` (`--resampleHeightWidthMax`) bound, or `null` when the source
 * already fits and the flag must be left off.
 *
 * ⚠ **Conditional, and it has to be.** `sips -Z` resamples *up* as well as
 * down — measured: a 64x32 PNG came back 8192x4096 and 3,500x its original
 * weight. Passing it unconditionally would inflate every ordinary photo past
 * the pixel cap the caller then applies, converting working previews into
 * refusals. It is only ever a ceiling for a source that exceeds one.
 */
function resampleBound(width: number, height: number): number | null {
  const longest = Math.max(width, height);
  const scale = Math.min(
    1,
    IMAGE_MAX_DIMENSION / longest,
    Math.sqrt(IMAGE_MAX_PIXELS / (width * height))
  );
  return scale < 1 ? Math.max(1, Math.floor(longest * scale)) : null;
}

/**
 * Converts a HEIC/HEIF file to PNG, returning the bytes.
 *
 * Returns `null` rather than throwing when conversion is unavailable or fails,
 * because the caller's fallback — an honest "this panel cannot decode HEIC"
 * card — is a better outcome than an error dialog.
 *
 * Every step is bounded because the input is attacker-controlled: artifacts are
 * auto-detected from assistant text and opened *without* a click, so a model
 * that names a hostile file is enough to reach this code. The declared extent
 * gates the spawn, `-Z` gates what the converter may write, and the size check
 * plus bounded read gate what this process — the one owning every window — will
 * hold in memory.
 */
export async function heicToPng(filePath: string): Promise<Buffer | null> {
  if (!(await canConvertHeic())) return null;

  const source = await heicDimensions(filePath);
  if (!source) return null;
  if (source.width * source.height > HEIC_MAX_SOURCE_PIXELS) {
    log.info('[heic] refusing to convert an image of', source.width, 'x', source.height);
    return null;
  }
  const bound = resampleBound(source.width, source.height);

  const outDir = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-heic-'));
  const outPath = path.join(outDir, 'preview.png');
  try {
    // Fixed argv through execFile — never a shell — so a filename containing
    // quotes or semicolons is an argument and not a command.
    await run(
      SIPS,
      [
        '-s',
        'format',
        'png',
        ...(bound === null ? [] : ['-Z', String(bound)]),
        filePath,
        '--out',
        outPath,
      ],
      { timeout: 20_000, windowsHide: true }
    );

    const handle = await fs.open(outPath, 'r');
    try {
      // `fstat` on the open handle, so what is measured is what is read.
      const { size } = await handle.stat();
      if (size > HEIC_PNG_MAX_BYTES) {
        throw new Error(`converted PNG is ${size} bytes, over the ${HEIC_PNG_MAX_BYTES} limit`);
      }
      return await readFileHandleBounded(handle, HEIC_PNG_MAX_BYTES);
    } finally {
      await handle.close();
    }
  } catch (error) {
    log.info('[heic] conversion failed:', error);
    return null;
  } finally {
    await fs.rm(outDir, { recursive: true, force: true }).catch(() => {});
  }
}
