import { createRoute } from "@tanstack/react-router"

import { WorkspacePage } from "../components/workspace/workspace-page"
import { authenticatedRoute } from "./_authenticated"

export const appRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/app",
  component: WorkspacePage,
})
