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
  resolve:
    mode === 'harness'
      ? {
          alias: {
            '@tauri-apps/api/core': fileURLToPath(
              new URL('./src/harness/tauriMock.ts', import.meta.url),
            ),
          },
        }
      : {},

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
