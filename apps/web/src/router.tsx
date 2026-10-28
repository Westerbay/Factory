import { createRouter } from "@tanstack/react-router"

import { rootRoute } from "./routes/__root"
import { indexRoute } from "./routes/index"

export const router = createRouter({
  routeTree: rootRoute.addChildren([indexRoute]),
  scrollRestoration: true,
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
