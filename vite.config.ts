import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const pkg = JSON.parse(readFileSync(resolve(import.meta.dirname, 'package.json'), 'utf-8'))

const deps = { ...pkg.dependencies, ...pkg.devDependencies }

export default defineConfig({
  base: '/',
  plugins: [tailwindcss(), react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __DEP_VERSIONS__: JSON.stringify(deps),
  },
  resolve: {
    alias: { '@': resolve(import.meta.dirname, 'src') },
  },
  // wasm-pack output ships its own .wasm via `new URL(..., import.meta.url)`;
  // keep it out of the dep optimizer so Vite resolves the URL itself.
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
})
