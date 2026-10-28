import { createRoute } from "@tanstack/react-router"

import { EnvironmentCard } from "../features/environments/environment-card"
import { authenticatedRoute } from "./_authenticated"

export const appRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/app",
  component: EnvironmentCard,
})
