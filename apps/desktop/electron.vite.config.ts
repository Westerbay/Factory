import react from "@vitejs/plugin-react"
import { defineConfig } from "electron-vite"

export default defineConfig({
  main: {},
  preload: {
    build: {
      rollupOptions: { output: { format: "cjs", entryFileNames: "index.cjs" } },
    },
  },
  renderer: { plugins: [react()], server: { host: "127.0.0.1" } },
})
