#!/usr/bin/env node
// Is a DevTools emulation override pinning the dev GUI's viewport?
//
//   node scripts/cdp-viewport-check.mjs [port]          # default 9333
//   npm run cdp:viewport-check -- 9333
//   npm run cdp:viewport-check -- 9333 --clear          # also try to clear it
//   npm run cdp:viewport-check -- 9333 --json
//
// Exit 0 = the viewport follows the window · 1 = PINNED · 2 = harness problem
// (no CDP endpoint, no page target).
//
// WHY THIS EXISTS. `agent-browser set_viewport`, Playwright's `setViewportSize`
// and DevTools device mode all apply `Emulation.setDeviceMetricsOverride`, which
// fixes the renderer's viewport regardless of the real window. Resizing the OS
// window then changes nothing on screen, a blank band opens below and to the
// right of the page, and every measurement taken afterwards is a lie. It reads
// exactly like a layout regression and is not one — see
// docs/desktop-ui/window-scaling-regressions.md, "Viewport emulation pins
// innerWidth". Measured on 2026-09-08 across two running instances:
// inner 1440×900 against outer 1638×963 and outer 1440×1000, with the layout
// correct throughout.
//
// THE TELL. macOS windows have no side frame, so `outerWidth` and `innerWidth`
// agree exactly on an unpinned window and any width difference at all is
// emulation; the height differs by the title bar (~28px). This script applies
// the same tolerances as the renderer's own dev-time warning
// (src/utils/viewportPin.ts) — 40px of title bar everywhere, plus 16px of frame
// on Windows and Linux.
//
// --clear IS NOT THE CURE, AND SAYS SO. Chromium keeps emulation state per
// DevTools session, so a session that never enabled emulation is a no-op on
// clear — hence the apply-then-clear dance below, which is what actually resets
// the widget. A foreign clear is UNRELIABLE, and both outcomes are measured:
// on the affected instance (2026-09-08) it restored `inner === outer` at once
// and left the renderer half-frozen — one more OS resize, then nothing, with
// `outerWidth` stale; on a fresh instance pinned and cleared the same way it
// recovered fully. You cannot tell from inside which you got, and the good
// outcome is the dangerous one because it looks fixed. A clean restart, with no
// emulation ever applied, tracked OS resizes exactly at 1200×800, 1900×1050 and
// 1440×1000 in both runs. **Restart the instance.**
//
// KNOWN BLIND SPOT. `inner` vs `outer` cannot see a pin applied at exactly the
// window size and then ORPHANED by a driver that detaches: an orphaned override
// freezes `outerWidth` too, so both numbers stop moving while still agreeing.
// While the setting session is attached, `outer` follows the window and the next
// resize exposes it. Neither real recurrence took that shape. The ground truth is
// the window manager — on macOS, compare against
//   osascript -e 'tell application "System Events" to tell (first process whose
//     unix id is <pid>) to get size of window 1'
// and remember to READ THE SIZE BACK after any resize you command.
//
// Node ≥ 22 (global `fetch` and `WebSocket`); no dependencies, deliberately —
// this has to run when the thing you are debugging is the tooling itself.

const args = process.argv.slice(2);
const port = args.find((a) => /^\d+$/.test(a)) ?? '9333';
const shouldClear = args.includes('--clear');
const asJson = args.includes('--json');

/** Height the window chrome may legitimately eat. Mirrors TITLE_BAR_SLACK_PX. */
const TITLE_BAR_SLACK_PX = 40;
/** Side/bottom frame allowance on Windows and Linux. Mirrors WINDOW_FRAME_SLACK_PX. */
const WINDOW_FRAME_SLACK_PX = 16;

const die = (code, message) => {
  console.error(message);
  process.exit(code);
};

let list;
try {
  list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
} catch (err) {
  die(2, `could not reach CDP on :${port} — is the dev GUI running there?\n${err.message}`);
}

const page = list.find(
  (t) => t.type === 'page' && !t.url.startsWith('chrome-extension') && !t.url.startsWith('devtools')
);
if (!page) die(2, `no page target on CDP port ${port}`);

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error(`could not open a CDP session on :${port}`));
}).catch((err) => die(2, err.message));

let nextId = 0;
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    const onMessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id !== id) return;
      ws.removeEventListener('message', onMessage);
      msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result);
    };
    ws.addEventListener('message', onMessage);
    ws.send(JSON.stringify({ id, method, params }));
  });

const measure = async () => {
  const { result } = await send('Runtime.evaluate', {
    expression: `JSON.stringify({
      inner: [innerWidth, innerHeight],
      outer: [outerWidth, outerHeight],
      client: [document.documentElement.clientWidth, document.documentElement.clientHeight],
      dpr: devicePixelRatio,
      platform: (window.electron && window.electron.platform) || 'unknown'
    })`,
    returnByValue: true,
  });
  return JSON.parse(result.value);
};

/**
 * The same signed comparison the renderer makes. Signed, not absolute: window
 * chrome only ever makes `outer` bigger, so a viewport LARGER than its window
 * can only have come from emulation, however small the difference.
 */
function diagnose(m) {
  const [iw, ih] = m.inner;
  const [ow, oh] = m.outer;
  if (![iw, ih, ow, oh].every((v) => Number.isFinite(v) && v > 0)) {
    return { pinned: false, note: 'a dimension read as zero — window hidden or minimized' };
  }
  const framed = m.platform !== 'darwin';
  const widthSlack = framed ? WINDOW_FRAME_SLACK_PX : 0;
  const heightSlack = TITLE_BAR_SLACK_PX + (framed ? WINDOW_FRAME_SLACK_PX : 0);
  const dw = ow - iw;
  const dh = oh - ih;
  const pinned = dw < 0 || dw > widthSlack || dh < 0 || dh > heightSlack;
  return { pinned, deltas: [dw, dh], slack: [widthSlack, heightSlack] };
}

const fmt = (m) =>
  `inner ${m.inner[0]}×${m.inner[1]}  outer ${m.outer[0]}×${m.outer[1]}  ` +
  `client ${m.client[0]}×${m.client[1]}  dpr ${m.dpr}  platform ${m.platform}`;

const before = await measure();
const verdict = diagnose(before);

let after = null;
if (shouldClear && verdict.pinned) {
  // Apply, then clear: a DevTools session that never enabled emulation is a
  // no-op on clear, so this session has to own an override before it can drop one.
  await send('Emulation.setDeviceMetricsOverride', {
    width: 0,
    height: 0,
    deviceScaleFactor: 0,
    mobile: false,
  });
  await send('Emulation.clearDeviceMetricsOverride');
  await new Promise((r) => setTimeout(r, 300)); // let the renderer relayout
  after = await measure();
}

if (asJson) {
  console.log(JSON.stringify({ port, title: page.title, before, after, ...verdict }, null, 2));
} else {
  console.log(`port ${port} (${page.title})`);
  console.log(`  ${fmt(before)}`);
  if (after) console.log(`  after clear: ${fmt(after)}`);
  if (verdict.note) console.log(`  ${verdict.note}`);
  if (verdict.pinned) {
    console.log('');
    console.log('PINNED: a DevTools device-metrics override is holding the viewport.');
    console.log(
      `  outer − inner = ${verdict.deltas[0]}×${verdict.deltas[1]}px, past the ` +
        `${verdict.slack[0]}×${verdict.slack[1]}px this platform's window chrome can explain.`
    );
    console.log('  The layout is NOT at fault. Do not go looking in the CSS.');
    console.log('');
    if (after) {
      console.log(
        'A clear was attempted from THIS session, and even when it restores inner === outer'
      );
      console.log(
        "it is not a fix: clearing another session's override was measured on 2026-09-08 to"
      );
      console.log(
        'leave the renderer HALF-FROZEN — it followed the next OS resize once and then stopped,'
      );
      console.log('with outerWidth stale.');
    }
    console.log('The sure cure is to RESTART the instance (no emulation ever applied), or to');
    console.log('have the driver session that set the override clear it before detaching.');
  } else {
    console.log('  viewport follows the window');
  }
}

ws.close();
process.exit(verdict.pinned ? 1 : 0);
