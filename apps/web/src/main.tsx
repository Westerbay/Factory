import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "@tanstack/react-router"

import { createQueryClient } from "./lib/query/query-client"
import { router } from "./router"
import { AppUpdates } from "./pwa/AppUpdates"
import "./lib/i18n"
import "./styles/app.css"
import "./lib/theme/theme"

const root = document.getElementById("root")
if (!root) throw new Error("Factory root element is missing")

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
      <AppUpdates />
    </QueryClientProvider>
  </StrictMode>
)
