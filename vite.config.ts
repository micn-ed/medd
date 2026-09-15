/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// https://v2.tauri.app/start/frontend/vite/
const host = process.env.TAURI_DEV_HOST

export default defineConfig(({ mode }) => ({
  plugins: [svelte()],

  // `npm run harness` swaps Tauri's IPC for an in-memory fixture so the frontend runs in an
  // ordinary browser and can be inspected by eye. See src/harness/tauriMock.ts for what this
  // does and does not prove. Never active in a normal dev or production build.
  resolve: {
    ...(mode === 'harness'
      ? {
          alias: {
            '@tauri-apps/api/core': fileURLToPath(
              new URL('./src/harness/tauriMock.ts', import.meta.url),
            ),
            '@tauri-apps/api/event': fileURLToPath(
              new URL('./src/harness/eventMock.ts', import.meta.url),
            ),
          },
        }
      : {}),
    // Vitest resolves Svelte's package exports with Node's default conditions, which picks
    // Svelte 5's server (SSR) build — @testing-library/svelte then calls a client-only mount API
    // that build doesn't have, and every component test fails with `lifecycle_function_unavailable`
    // before it renders anything. Forcing the `browser` condition under Vitest (never in a normal
    // dev server or production build, where Vite already resolves this correctly) is what makes
    // component tests exercise the actual client-side Svelte runtime the app ships.
    ...(process.env.VITEST ? { conditions: ['browser'] } : {}),
  },

  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test-setup.ts'],
  },

  // Prevent Vite from obscuring Rust errors.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || '127.0.0.1',
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
}))
