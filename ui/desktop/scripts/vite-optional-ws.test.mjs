import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { build } from 'vite';

const require = createRequire(import.meta.url);
const ViteConfigGenerator = require('@electron-forge/plugin-vite/dist/ViteConfig').default;
const project = fileURLToPath(new URL('..', import.meta.url));
const optionalPeers = new Set(['bufferutil', 'utf-8-validate']);

for (const production of [false, true]) {
  test(
    `Forge ${production ? 'production' : 'development'} ws bundle uses the absent-peer JS fallback`,
    { timeout: 30000 },
    async () => {
      const root = await mkdtemp(path.join(os.tmpdir(), 'biorouter-ws-bundle-'));
      let client;
      let server;
      try {
        const entry = path.join(root, 'entry.mjs');
        await writeFile(
          entry,
          `export { default as WebSocket, WebSocketServer } from ${JSON.stringify(require.resolve('ws'))};\n`
        );
        const generator = new ViteConfigGenerator(
          {
            build: [{ entry, config: path.join(project, 'vite.main.config.mts'), target: 'main' }],
            renderer: [],
          },
          project,
          production
        );
        const [config] = await generator.getBuildConfigs();
        const result = await build({
          ...config,
          configFile: false,
          logLevel: 'error',
          build: { ...config.build, watch: null, write: false },
        });
        const results = Array.isArray(result) ? result : [result];
        assert.ok(results.every((output) => 'output' in output));
        const entries = results
          .flatMap((output) => output.output)
          .filter((output) => output.type === 'chunk' && output.isEntry);
        assert.equal(entries.length, 1);
        const [chunk] = entries;
        assert.doesNotMatch(chunk.code, /__viteOptionalPeerDep|__vite-optional-peer-dep/);
        const attempts = [];
        const module = { exports: {} };
        vm.runInNewContext(
          chunk.code,
          {
            module,
            exports: module.exports,
            Buffer,
            URL,
            setTimeout,
            clearTimeout,
            setImmediate,
            clearImmediate,
            process: { env: {}, version: process.version, nextTick: process.nextTick },
            require(id) {
              if (optionalPeers.has(id)) {
                attempts.push(id);
                const error = new Error('Optional test peer is absent');
                error.code = 'MODULE_NOT_FOUND';
                throw error;
              }
              return require(id);
            },
          },
          { filename: 'ws-bundle.cjs' }
        );
        const { WebSocket, WebSocketServer } = module.exports;
        assert.ok(attempts.includes('bufferutil'));
        server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
        await once(server, 'listening');
        server.on('connection', (socket) => {
          socket.on('message', (data) => socket.send(data));
        });
        client = new WebSocket(`ws://127.0.0.1:${server.address().port}`);
        await once(client, 'open');
        const payload = Buffer.from('fallback-✓'.repeat(1024));
        const echoed = once(client, 'message');
        client.send(payload);
        const [message] = await echoed;
        assert.deepEqual(message, payload);
      } finally {
        client?.terminate();
        if (server) {
          for (const socket of server.clients) socket.terminate();
          await new Promise((resolve) => server.close(resolve));
        }
        await rm(root, { recursive: true, force: true });
      }
    }
  );
}
