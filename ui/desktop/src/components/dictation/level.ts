/**
 * A chunk's RMS on the meter's 0 to 1 scale: -60 dBFS (the noise floor the
 * session's MSG-NO-AUDIO-YET uses) to full scale. The recording pill's bars, the
 * on-screen indicator and the hidden Dictate to clipboard page all read it.
 */
export function levelToMeter(rms: number): number {
  if (!(rms > 0)) return 0;
  const db = 20 * Math.log10(rms);
  return Math.max(0, Math.min(1, (db + 60) / 60));
}
