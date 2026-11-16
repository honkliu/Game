import { defineConfig } from 'vitest/config';
import preact from '@preact/preset-vite';
import { fileURLToPath } from 'node:url';

const out = fileURLToPath(new URL('../crates/shell/ui/sidebar', import.meta.url));

// The built pane is served by the browser's `asset://` protocol and embedded in
// browser.exe, so file names are stable (no content hashes) and paths are relative.
export default defineConfig({
  plugins: [preact()],
  base: './',
  build: {
    outDir: out,
    emptyOutDir: true,
    target: 'es2022',
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      input: fileURLToPath(new URL('./sidebar.html', import.meta.url)),
      output: {
        entryFileNames: 'sidebar.js',
        chunkFileNames: 'chunk-[name].js',
        assetFileNames: '[name][extname]',
      },
    },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts', 'tests/**/*.test.tsx'],
  },
});
