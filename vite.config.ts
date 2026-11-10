import { defineConfig } from 'vite';

// Tauri expects a fixed dev-server port and the WebView2 (Chromium) build target.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'chrome120',
    outDir: 'dist',
    emptyOutDir: true,
  },
});
