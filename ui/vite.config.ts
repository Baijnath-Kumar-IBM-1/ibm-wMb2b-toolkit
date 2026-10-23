import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'

// Dev server and preview bind to loopback only.
//
// Development:
//   trace-server --port 8787 --allow-origin http://127.0.0.1:5173 \
//     --token <at least 32 characters>
//   pnpm dev            (TRACE_API_PORT overrides the default port 8787)
//   open http://127.0.0.1:5173/#token=<token>
//
// The proxy keeps the UI same-origin and rewrites `Host` to the API server's,
// which its DNS-rebinding check requires. The browser's `Origin` is passed
// through and must be listed with --allow-origin.
export default defineConfig(({ mode }) => {
  const apiPort = loadEnv(mode, '.', 'TRACE_').TRACE_API_PORT ?? '8787'
  return {
    plugins: [react()],
    server: {
      host: '127.0.0.1',
      port: 5173,
      strictPort: true,
      proxy: {
        '/api': {
          target: `http://127.0.0.1:${apiPort}`,
          changeOrigin: true,
        },
      },
    },
    preview: {
      host: '127.0.0.1',
      port: 4173,
      strictPort: true,
    },
  }
})
