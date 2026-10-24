import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The .NET server owns /api and /hubs. In dev, Vite proxies to it so the
// browser sees a single origin (which the CSRF check requires).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': { target: 'http://localhost:5199', changeOrigin: false },
      '/hubs': { target: 'http://localhost:5199', ws: true, changeOrigin: false }
    }
  },
  build: {
    outDir: '../server/CityOfTowers/wwwroot',
    emptyOutDir: true
  }
});