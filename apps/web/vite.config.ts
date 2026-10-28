import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

import { parseApiOrigin } from "./src/lib/api/config.ts"

export default defineConfig({
  // Configuration comes from explicit process variables, never local secret files.
  envDir: false,
  plugins: [react(), tailwindcss()],
  define: {
    "import.meta.env.VITE_API_BASE_URL": JSON.stringify(
      parseApiOrigin(process.env.VITE_API_BASE_URL)
    ),
  },
  server: { host: "127.0.0.1", port: 3000, strictPort: true },
  preview: { host: "127.0.0.1", port: 3000, strictPort: true },
})
