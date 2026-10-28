import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"
import { VitePWA } from "vite-plugin-pwa"

import { parseApiOrigin } from "./src/lib/api/config.ts"

export default defineConfig({
  // Configuration comes from explicit process variables, never local secret files.
  envDir: false,
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      strategies: "injectManifest",
      srcDir: "src/pwa",
      filename: "sw.ts",
      injectRegister: false,
      registerType: "prompt",
      includeManifestIcons: false,
      injectManifest: {
        globPatterns: ["assets/**/*.{js,css}", "icons/*.png", "offline.html"],
      },
      manifest: {
        id: "/",
        name: "Factory",
        short_name: "Factory",
        description: "Your private workspace",
        start_url: "/",
        scope: "/",
        display: "standalone",
        background_color: "#f7f8fa",
        theme_color: "#5340ac",
        icons: [
          { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png" },
          { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png" },
          {
            src: "/icons/maskable-512.png",
            sizes: "512x512",
            type: "image/png",
            purpose: "maskable",
          },
        ],
      },
    }),
  ],
  define: {
    "import.meta.env.VITE_API_BASE_URL": JSON.stringify(
      parseApiOrigin(process.env.VITE_API_BASE_URL)
    ),
  },
  server: {
    host: "127.0.0.1",
    port: 3000,
    strictPort: true,
    headers: { "Cache-Control": "no-store" },
  },
  preview: {
    host: "127.0.0.1",
    port: 3000,
    strictPort: true,
    headers: { "Cache-Control": "no-store" },
  },
})
