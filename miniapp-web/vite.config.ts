import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The program embeds the build (lib/securitycamera/dist) and serves it,
// so the machine running the camera does not need Node.
export default defineConfig({
  plugins: [react()],
  build: { outDir: '../lib/securitycamera/dist', emptyOutDir: true },
  // pnpm run dev: the API calls go to the program running locally
  server: { proxy: { '/api': 'http://127.0.0.1:8081', '/stream': 'http://127.0.0.1:8081', '/events': 'http://127.0.0.1:8081' } },
  test: { environment: 'node' },
})
