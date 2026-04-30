import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig(({ command }) => ({
  // Production is served from labs.golia.jp/aarch64; dev runs at the host root.
  base: command === 'build' ? '/aarch64/' : '/',
  plugins: [tailwindcss(), react()],
  // wasm-pack ships its own .wasm via `new URL(..., import.meta.url)`; keep
  // it out of Vite's dep optimiser so Vite resolves the URL itself.
  optimizeDeps: {
    exclude: ['aarch64-sim'],
  },
  server: {
    host: '127.0.0.1',
    port: 32030,
    strictPort: true,
  },
  preview: {
    port: 32030,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
  },
}))
