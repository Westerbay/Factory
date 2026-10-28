import { createRouter } from "@tanstack/react-router"

import { rootRoute } from "./routes/__root"
import { indexRoute } from "./routes/index"
import { signInRoute } from "./routes/sign-in"
import { authenticatedRoute } from "./routes/_authenticated"
import { appRoute } from "./routes/_authenticated.app"

export const router = createRouter({
  routeTree: rootRoute.addChildren([
    indexRoute,
    signInRoute,
    authenticatedRoute.addChildren([appRoute]),
  ]),
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
