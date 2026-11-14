import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import { fontScale } from './vite-plugins/fontScale.mjs';
import { pdfjsAssets } from './vite-plugins/pdfjsAssets.mjs';

// https://vitejs.dev/config
export default defineConfig({
  define: {
    // This replaces process.env.ALPHA with a literal at build time
    'process.env.ALPHA': JSON.stringify(process.env.ALPHA === 'true'),
    'process.env.BIOROUTER_TUNNEL': JSON.stringify(
      process.env.BIOROUTER_TUNNEL !== 'no' && process.env.BIOROUTER_TUNNEL !== 'none'
    ),
  },

  plugins: [tailwindcss(), pdfjsAssets()],
  css: { postcss: { plugins: [fontScale()] } },

  // BIOROUTER_NO_HMR=1 freezes the renderer: no watching, no hot reload. Set it
  // when driving the dev GUI (agent-browser / Playwright) while the tree is
  // being edited, otherwise every save full-reloads the page and destroys the
  // chat session under test.
  server: process.env.BIOROUTER_NO_HMR ? { hmr: false, watch: { ignored: ['**'] } } : undefined,

  optimizeDeps: {
    entries: ['index.html'],
  },

  build: {
    target: 'esnext',
    // Dictation's AudioWorklet module must stay a real file (DD-64). Vite inlines
    // assets under 4,096 B as `data:` URLs, and the renderer's content security
    // policy blocks a `data:` worklet, so dictation would fail on the first click
    // with "Unable to load a worklet's module". The import in
    // `components/dictation/capture.ts` also says `?url&no-inline`; this keeps any
    // other `*.worklet.js` from meeting the same fate. Everything else keeps
    // Vite's default limit (`undefined`).
    assetsInlineLimit: (filePath: string) =>
      /\.worklet\.js(?:$|\?)/.test(filePath) ? false : undefined,
  },
});
