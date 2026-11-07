// Biorouter dictation: microphone to 16-bit PCM (DD-64).
// Loaded only through capture.ts as `?url&no-inline`; keep it a plain file
// under 4 KiB, the size below which Vite would otherwise inline it as a data:
// URL that the content security policy blocks.
//
// Every render quantum's samples are posted as they are made, stamped with the
// context frame they start at (`currentFrame`), so the page already holds all
// but the last few milliseconds when the person stops, and the stop never waits
// for an answer from here: after an input event Chromium renders a frame before
// it delivers this thread's messages, which cost up to 10 ms on the stop path
// when the stop asked for the last partial chunk. The stamps tell the page how
// much audio recorded before the stop is still queued behind a busy main
// thread. The page assembles the 100 ms chunks itself. A `flush` still gets one
// `final` message, after every chunk, so the page knows this processor ended.
class BiorouterPcmCapture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.live = false;
    this.done = false;
    this.port.onmessage = (event) => {
      if (event.data && event.data.type === 'flush' && !this.done) {
        this.done = true;
        const empty = new Int16Array(0);
        this.port.postMessage({ type: 'chunk', pcm: empty.buffer, final: true }, [empty.buffer]);
      }
    };
  }

  process(inputs) {
    if (this.done) return false;
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    if (!this.live) {
      this.live = true;
      this.port.postMessage({ type: 'live', frame: currentFrame });
    }
    const channels = input.length;
    const frames = input[0].length;
    const pcm = new Int16Array(frames);
    for (let i = 0; i < frames; i++) {
      let s = 0;
      for (let c = 0; c < channels; c++) s += input[c][i];
      s /= channels;
      // The resampler overshoots peaks by up to about 5 %, hence the clamp.
      if (s > 1) s = 1;
      else if (s < -1) s = -1;
      pcm[i] = Math.round(s * 32767);
    }
    this.port.postMessage({ type: 'chunk', pcm: pcm.buffer, frame: currentFrame, final: false }, [
      pcm.buffer,
    ]);
    return true;
  }
}

registerProcessor('biorouter-pcm-capture', BiorouterPcmCapture);
