import { defineConfig } from "electron-vite"
export default defineConfig({
  main: {
    build: {
      outDir: "out/smoke-main",
      rollupOptions: {
        input: { smoke: "test/smoke.ts", worker: "test/smoke-worker.ts" },
      },
    },
  },
})
