import { defineConfig } from 'vite'
import solid from 'vite-plugin-solid'
import tailwindcss from '@tailwindcss/vite'

/**
 * The backend listens on 127.0.0.1:3000 by default. `pnpm dev` serves the SPA
 * itself and forwards everything the Rust server owns, so the app talks to
 * same-origin paths in dev exactly as it does when the binary serves `dist/`.
 */
const BACKEND = 'http://127.0.0.1:3000'

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  server: {
    proxy: {
      '/api': { target: BACKEND, changeOrigin: true },
      '/api-docs': { target: BACKEND, changeOrigin: true },
      '/swagger-ui': { target: BACKEND, changeOrigin: true },
    },
  },
  // `dist/` is what the Rust backend serves via ServeDir; leave it at the
  // Vite default.
  build: {
    outDir: 'dist',
  },
})
