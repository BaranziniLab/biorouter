// Biorouter dictation: microphone to 16-bit PCM chunks (DD-64).
// Loaded only through capture.ts as `?url&no-inline`; keep it a plain file
// under 4 KiB, the size below which Vite would otherwise inline it as a data:
// URL that the content security policy blocks.
class BiorouterPcmCapture extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const frames = options && options.processorOptions && options.processorOptions.chunkFrames;
    this.size = frames > 0 ? frames : 1600;
    this.buf = new Int16Array(this.size);
    this.fill = 0;
    this.sum = 0;
    this.peak = 0;
    this.live = false;
    this.done = false;
    this.port.onmessage = (event) => {
      if (event.data && event.data.type === 'flush' && !this.done) {
        this.done = true;
        this.post(true);
      }
    };
  }

  post(final) {
    const pcm = this.buf.slice(0, this.fill);
    const rms = this.fill > 0 ? Math.sqrt(this.sum / this.fill) : 0;
    this.port.postMessage({ type: 'chunk', pcm: pcm.buffer, rms, peak: this.peak, final }, [
      pcm.buffer,
    ]);
    this.fill = 0;
    this.sum = 0;
    this.peak = 0;
  }

  process(inputs) {
    if (this.done) return false;
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    if (!this.live) {
      this.live = true;
      this.port.postMessage({ type: 'live' });
    }
    const channels = input.length;
    const frames = input[0].length;
    for (let i = 0; i < frames; i++) {
      let s = 0;
      for (let c = 0; c < channels; c++) s += input[c][i];
      s /= channels;
      // The resampler overshoots peaks by up to about 5 %, hence the clamp.
      if (s > 1) s = 1;
      else if (s < -1) s = -1;
      this.sum += s * s;
      const a = s < 0 ? -s : s;
      if (a > this.peak) this.peak = a;
      this.buf[this.fill++] = Math.round(s * 32767);
      if (this.fill === this.size) this.post(false);
    }
    return true;
  }
}

registerProcessor('biorouter-pcm-capture', BiorouterPcmCapture);
