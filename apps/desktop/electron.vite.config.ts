import { randomBytes } from "node:crypto"
import type { Plugin } from "vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "electron-vite"

export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        input: { index: "src/main/index.ts", worker: "src/runtime/worker.ts" },
      },
    },
  },
  preload: {
    build: {
      externalizeDeps: false,
      rollupOptions: { output: { format: "cjs", entryFileNames: "index.cjs" } },
    },
  },
  renderer: {
    plugins: [react(), developmentCsp()],
    server: { host: "127.0.0.1" },
  },
})

// Fast Refresh uses a trusted inline preamble. Give only development HTML a fresh nonce.
function developmentCsp(): Plugin {
  const nonce = randomBytes(24).toString("base64")
  return {
    name: "factory-development-csp",
    apply: "serve",
    transformIndexHtml: {
      order: "post",
      handler(html) {
        return html
          .replace("script-src 'self'", `script-src 'self' 'nonce-${nonce}'`)
          .replace("style-src 'self'", "style-src 'self' 'unsafe-inline'")
          .replace("connect-src 'none'", "connect-src 'self' ws://127.0.0.1:*")
          .replace(/<script(?![^>]*\bsrc=)/g, `<script nonce="${nonce}"`)
      },
    },
  }
}
