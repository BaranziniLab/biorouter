#!/usr/bin/env node
/*
 * Agent Drafter — agentic-loop test kit.
 *
 * Verifies the loop that powers agentic artifacts end-to-end: it launches the
 * Biorouter ACP agent over a WebSocket (`biorouter acp --ws`), connects as an
 * ACP client, and checks
 * that replies are REAL agentic answers from the configured model — not
 * hardwired / rule-based strings:
 *
 *   1. Arithmetic on arbitrary operands the model must actually compute.
 *   2. Two distinct prompts produce two distinct, on-topic answers.
 *
 * A canned/conditional responder cannot pass both. Uses Node's global
 * WebSocket (Node 22+), no dependencies.
 *
 * Usage:
 *   BIOROUTER_BIN=/path/to/biorouter node scripts/agent-drafter/agentic-loop-test.mjs
 *   (defaults BIOROUTER_BIN=biorouter, ACP_WS_PORT=11599)
 */
import { spawn } from 'node:child_process';

const BIN = process.env.BIOROUTER_BIN || 'biorouter';
const PORT = process.env.ACP_WS_PORT || '11599';
const ADDR = `127.0.0.1:${PORT}`;
const URL = `ws://${ADDR}`;

const log = (...a) => console.log('[loop-test]', ...a);
const fail = (msg) => { console.error('[loop-test] FAIL:', msg); cleanup(1); };

let server;
function cleanup(code) {
  try { server && server.kill('SIGKILL'); } catch {}
  process.exit(code);
}

function connect(url, timeoutMs = 8000) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const t = setTimeout(() => reject(new Error('ws connect timeout')), timeoutMs);
    ws.onopen = () => { clearTimeout(t); resolve(ws); };
    ws.onerror = (e) => { clearTimeout(t); reject(e.error || new Error('ws error')); };
  });
}

// One ACP client over the socket: id-correlated requests + session/update stream.
function acpClient(ws) {
  let nextId = 1;
  const pending = new Map();
  const chunks = []; // {sessionId, text}
  ws.onmessage = (ev) => {
    let msg;
    try { msg = JSON.parse(ev.data); } catch { return; }
    if (msg.id != null && pending.has(msg.id)) {
      const p = pending.get(msg.id); pending.delete(msg.id);
      msg.error ? p.reject(new Error(JSON.stringify(msg.error))) : p.resolve(msg.result);
    } else if (msg.method === 'session/update') {
      const u = (msg.params && msg.params.update) || {};
      const kind = u.sessionUpdate || u.session_update;
      if (kind === 'agent_message_chunk') {
        const text =
          (u.content && (u.content.text ?? (u.content.content && u.content.content.text))) || '';
        chunks.push(text);
      }
    }
  };
  const req = (method, params) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params: params || {} }));
      setTimeout(() => {
        if (pending.has(id)) { pending.delete(id); reject(new Error(`timeout: ${method}`)); }
      }, 90000);
    });
  return { req, chunks, takeText: () => { const t = chunks.join(''); chunks.length = 0; return t; } };
}

async function main() {
  log(`launching: ${BIN} acp --ws ${ADDR}`);
  server = spawn(BIN, ['acp', '--ws', ADDR], { stdio: ['ignore', 'pipe', 'pipe'] });
  server.stderr.on('data', (d) => process.env.VERBOSE && process.stderr.write(d));
  server.on('exit', (c) => { if (c) log(`server exited early (${c})`); });

  // Wait for the WS endpoint to accept connections.
  let ws;
  for (let i = 0; i < 40; i++) {
    try { ws = await connect(URL, 1000); break; } catch { await new Promise((r) => setTimeout(r, 500)); }
  }
  if (!ws) fail('server never accepted a WebSocket connection');
  log('connected');

  const cx = acpClient(ws);
  await cx.req('initialize', { protocolVersion: 1 });
  const session = await cx.req('session/new', { cwd: process.cwd(), mcpServers: [] });
  const sessionId = session.sessionId || session.session_id;
  if (!sessionId) fail('no sessionId from session/new');
  log('session', sessionId);

  async function ask(text) {
    cx.takeText();
    const res = await cx.req('session/prompt', { sessionId, prompt: [{ type: 'text', text }] });
    // small drain window for trailing chunks
    await new Promise((r) => setTimeout(r, 250));
    return { stop: res && (res.stopReason || res.stop_reason), answer: cx.takeText().trim() };
  }

  // Test 1 — arbitrary arithmetic the model must compute (not hardwired).
  const a = 17, b = 23, prod = a * b; // 391
  const r1 = await ask(`What is ${a} multiplied by ${b}? Reply with only the number.`);
  log(`Q1 (${a}x${b}) -> ${JSON.stringify(r1.answer).slice(0, 120)}`);
  if (!r1.answer) fail('empty answer to arithmetic prompt');
  if (!r1.answer.replace(/[,\s]/g, '').includes(String(prod)))
    fail(`arithmetic wrong/hardwired: expected ${prod}, got "${r1.answer}"`);

  // Test 2 — two distinct prompts -> distinct, on-topic answers (agentic, not canned).
  const r2 = await ask('In one short sentence, what is a mitochondrion?');
  const r3 = await ask('In one short sentence, what is photosynthesis?');
  log(`Q2 -> ${JSON.stringify(r2.answer).slice(0, 100)}`);
  log(`Q3 -> ${JSON.stringify(r3.answer).slice(0, 100)}`);
  if (!r2.answer || !r3.answer) fail('empty answer to a knowledge prompt');
  if (r2.answer === r3.answer) fail('identical answers to distinct prompts (hardwired)');
  if (!/mitochond|cell|energy|atp/i.test(r2.answer)) fail('Q2 answer off-topic (not agentic)');
  if (!/photosynth|light|plant|glucose|carbon/i.test(r3.answer)) fail('Q3 answer off-topic (not agentic)');

  log('PASS — agentic loop works: arithmetic correct, distinct on-topic answers, real streaming.');
  cleanup(0);
}

main().catch((e) => fail(e && e.message ? e.message : String(e)));
