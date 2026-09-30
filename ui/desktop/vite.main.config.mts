import { defineConfig } from 'vite';

// https://vitejs.dev/config
export default defineConfig({
  build: {
    rollupOptions: {
      // Keep ws's optional native requires inside its own try/catch fallback.
      external: ['node-pty', 'bufferutil', 'utf-8-validate'],
    },
  },
});
